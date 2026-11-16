# tools/testclient - scenario orchestrator (stage V).
#
# Runs the visibility scenarios S1-S7 against the vanilla test stand:
# vanilla server (E:\MineRust-data\vanilla; RCON 25575 for existing scenarios) behind mcproxy
# (127.0.0.1:25565 -> 25566, traces in <runDir>\mcproxy) with Fabric test
# clients launched already connected (quickPlay; the mod's `connect` is
# blocked, see README). Scenarios never reconnect inside one client
# session: relaunching a client = kill process + quickPlay again.
#
# Usage:
#   .\run-scenario.ps1                    # all scenarios, fresh runDir
#   .\run-scenario.ps1 -Scenarios s3      # one scenario
#   .\run-scenario.ps1 -Target ours       # stage G: our Rust server behind
#                                         # the same mcproxy instead of vanilla
#   .\run-scenario.ps1 -KeepStand         # leave stand+mcproxy running
#
# Results: <runDir>\summary.json, S7 facts in <runDir>\summary.txt, traces under <runDir>\mcproxy.
# The stand is always torn down in finally (own vanilla/mcproxy/clients).
[CmdletBinding()]
param(
	[string[]]$Scenarios = @('s1', 's1b', 's2', 's3', 's4', 's5', 's7'),
	[string]$RunDir = '',
	[ValidateSet('vanilla', 'ours')]
	[string]$Target = 'vanilla',
	# Game mode new players join in on our server (Шаг 14c). -1 (the default)
	# leaves the generated server-ours.toml untouched (survival). The S9
	# creative run uses 1: our server has no RCON to switch modes in game.
	[int]$OursGameMode = -1,
	# Development-only spawn override for our server (Шаг 14d): "x,y,z" is
	# written into the generated server-ours.toml as spawn_override. Empty
	# (the default) leaves the config untouched (the save's spawn). S8 uses it
	# to place the joining player above open water, since our server has no
	# RCON to teleport the player in game.
	[string]$OursSpawnOverride = '',
	[switch]$KeepStand
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'scenario-lib.ps1')

# --- paths -----------------------------------------------------------------
$vanillaDir = 'E:\MineRust-data\vanilla'
$templateWorldDir = 'E:\MineRust-data\world-template-v3'
$script:activeWorldDir = $null
$serverJar = 'E:\MineRust-data\server.jar'
$mcproxyExe = 'E:\projects\MineRust\target\debug\mcproxy.exe'
# Stage G target: our own Rust server (workspace server.toml, listen patched
# to 25566 because mcproxy reads its upstream port from the vanilla
# server.properties).
$serverExe = 'E:\projects\MineRust\target\debug\server.exe'
$repoServerToml = 'E:\projects\MineRust\server.toml'
$isVanilla = ($Target -eq 'vanilla')
if (-not $RunDir) { $RunDir = Join-Path 'E:\MineRust-data\autotests' (Get-Date -Format 'yyyy-MM-dd_HHmm') }
New-Item -ItemType Directory -Path $RunDir -Force | Out-Null
$traceDir = Join-Path $RunDir 'mcproxy'
New-Item -ItemType Directory -Path $traceDir -Force | Out-Null
$script:runStartedAt = Get-Date
$script:consoleReady = $false

function Find-EntitySpawn {
	param($Rows, [byte[]]$UuidBytes)
	foreach ($row in $Rows) {
		if ($row.dir -ne 'clientbound' -or $row.state -ne 'play' -or $row.name -ne 'minecraft:add_entity') { continue }
		$payload = ConvertFrom-TraceB64 $row.b64
		$entityId = 0
		$offset = 0
		$valid = $false
		for ($part = 0; $part -lt 5 -and $offset -lt $payload.Length; $part++) {
			$byte = [int]$payload[$offset]
			$entityId = $entityId -bor (($byte -band 0x7f) -shl (7 * $part))
			$offset++
			if (($byte -band 0x80) -eq 0) { $valid = $true; break }
		}
		if (-not $valid -or $payload.Length -lt ($offset + 16)) { continue }
		$matches = $true
		for ($i = 0; $i -lt 16; $i++) {
			if ($payload[$offset + $i] -ne $UuidBytes[$i]) { $matches = $false; break }
		}
		if ($matches) {
			return [PSCustomObject]@{ row = $row; entityId = $entityId }
		}
	}
	return $null
}
$vanillaLog = Join-Path $RunDir 'vanilla-stdout.log'
$oursLog = Join-Path $RunDir 'ours-stdout.log'
$rconScript = Join-Path $here 'Invoke-Rcon.ps1'
$launchScript = Join-Path $here 'launch-fabric-client.ps1'
$summaryPath = Join-Path $RunDir 'summary.json'
$summaryTextPath = Join-Path $RunDir 'summary.txt'
$script:s7Facts = New-Object System.Collections.Generic.List[string]
$script:vanillaProcess = $null

# Stand PIDs, cleaned in finally.
$script:vanillaPid = $null
$script:oursPid = $null
$script:mcproxyPid = $null

function Get-WorldSnapshot {
	param([string]$WorldDir)
	$files = @(Get-ChildItem -LiteralPath $WorldDir -File -Recurse -Force | ForEach-Object {
		$relative = $_.FullName.Substring($WorldDir.Length).TrimStart('\')
		[PSCustomObject]@{
			path = $relative
			length = $_.Length
			lastWriteTime = $_.LastWriteTime.ToString('o')
			sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
		}
	})
	$playerDataDir = Join-Path $WorldDir 'players\data'
	$playerData = @()
	if (Test-Path -LiteralPath $playerDataDir) {
		$playerData = @(Get-ChildItem -LiteralPath $playerDataDir -File -Filter '*.dat' -Force |
			ForEach-Object { $_.Name } | Sort-Object)
	}
	return [PSCustomObject]@{ files = $files; playerData = $playerData }
}

function Assert-WorldPlayerData {
	param([string[]]$WorldPlayerData, [string[]]$TemplatePlayerData)
	$unexpected = @($WorldPlayerData | Where-Object { $_ -notin $TemplatePlayerData })
	if ($unexpected.Count -gt 0) {
		throw ("world contains playerdata absent from template; refusing client launch: {0}" -f ($unexpected -join ', '))
	}
	return $true
}

# Removes old world.stale-<timestamp> siblings of the active world, keeping
# the newest $Keep. Path check before every delete: the leaf must match
# '<active leaf>.stale-<yyyyMMdd-HHmmss-fff>', the resolved parent must be the
# active world's parent, and the target must be neither the template nor the
# active world itself.
function Prune-StaleWorlds {
	param([string]$ActiveWorldDir, [int]$Keep = 2)
	$leaf = Split-Path -Leaf $ActiveWorldDir
	$parent = (Resolve-Path -LiteralPath (Split-Path -Parent $ActiveWorldDir)).Path
	$pattern = '^' + [regex]::Escape($leaf) + '\.stale-\d{8}-\d{6}-\d{3}$'
	$stampPrefix = $leaf + '.stale-'
	$stale = @(Get-ChildItem -LiteralPath $parent -Directory | Where-Object { $_.Name -match $pattern })
	if ($stale.Count -le $Keep) { return }
	$inv = [System.Globalization.CultureInfo]::InvariantCulture
	$ordered = $stale | Sort-Object -Property @{ Expression = {
		[datetime]::ParseExact($_.Name.Substring($stampPrefix.Length), 'yyyyMMdd-HHmmss-fff', $inv)
	} } -Descending
	foreach ($dir in ($ordered | Select-Object -Skip $Keep)) {
		$full = (Resolve-Path -LiteralPath $dir.FullName).Path
		if ((Split-Path -Leaf $full) -notmatch $pattern) {
			throw "refusing to prune: stale world name does not match the pattern: $full"
		}
		if (-not $full.StartsWith($parent + [System.IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
			throw "refusing to prune: stale world escaped the world parent: $full"
		}
		Remove-Item -LiteralPath $full -Recurse -Force
	}
}

function Prepare-RunWorld {
	if (-not (Test-Path -LiteralPath $templateWorldDir -PathType Container)) {
		throw "world template does not exist: $templateWorldDir"
	}
	$script:activeWorldDir = if ($isVanilla) {
		Join-Path $vanillaDir 'world'
	} else {
		Join-Path (Split-Path -Parent $templateWorldDir) 'world'
	}
	if ([string]::Equals($script:activeWorldDir, $templateWorldDir, [StringComparison]::OrdinalIgnoreCase)) {
		throw 'refusing to replace the source world template'
	}
	if (Test-Path -LiteralPath $script:activeWorldDir) {
		$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
		$staleDir = "$($script:activeWorldDir).stale-$stamp"
		if (Test-Path -LiteralPath $staleDir) { throw "stale world destination already exists: $staleDir" }
		Rename-Item -LiteralPath $script:activeWorldDir -NewName (Split-Path -Leaf $staleDir)
	}
	Prune-StaleWorlds -ActiveWorldDir $script:activeWorldDir -Keep 2
	New-Item -ItemType Directory -Path $script:activeWorldDir | Out-Null
	Get-ChildItem -LiteralPath $templateWorldDir -Force | Copy-Item -Destination $script:activeWorldDir -Recurse -Force

	$snapshot = Get-WorldSnapshot $script:activeWorldDir
	$templatePlayerDataDir = Join-Path $templateWorldDir 'players\data'
	$templatePlayerData = @()
	if (Test-Path -LiteralPath $templatePlayerDataDir) {
		$templatePlayerData = @(Get-ChildItem -LiteralPath $templatePlayerDataDir -File -Filter '*.dat' -Force |
			ForEach-Object { $_.Name } | Sort-Object)
	}
	$unexpected = @($snapshot.playerData | Where-Object { $_ -notin $templatePlayerData })
	$audit = [PSCustomObject]@{
		capturedAt = (Get-Date).ToString('o')
		target = $Target
		worldPath = $script:activeWorldDir
		templatePath = $templateWorldDir
		files = $snapshot.files
		playerData = $snapshot.playerData
		templatePlayerData = $templatePlayerData
		unexpectedPlayerData = $unexpected
	}
	$auditPath = Join-Path $RunDir 'world-audit.json'
	$audit | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $auditPath -Encoding utf8
	Write-Host "world audit: $auditPath"
	Assert-WorldPlayerData $snapshot.playerData $templatePlayerData | Out-Null
}

function Start-Stand {
	if ($isVanilla) {
		$startInfo = New-Object System.Diagnostics.ProcessStartInfo
		$startInfo.FileName = 'java'
		$startInfo.Arguments = "-Xmx1G -jar `"$serverJar`" nogui"
		$startInfo.WorkingDirectory = $vanillaDir
		$startInfo.UseShellExecute = $false
		$startInfo.RedirectStandardInput = $true
		$startInfo.RedirectStandardOutput = $true
		$startInfo.RedirectStandardError = $true
		$script:vanillaProcess = New-Object System.Diagnostics.Process
		$script:vanillaProcess.StartInfo = $startInfo
		if (-not $script:vanillaProcess.Start()) { throw 'could not start vanilla server' }
		$script:vanillaPid = $script:vanillaProcess.Id
		$script:vanillaOutputEvent = Register-ObjectEvent -InputObject $script:vanillaProcess `
			-EventName OutputDataReceived -MessageData $vanillaLog -Action {
				if ($EventArgs.Data) { Add-Content -LiteralPath $Event.MessageData -Value $EventArgs.Data }
			}
		$script:vanillaErrorEvent = Register-ObjectEvent -InputObject $script:vanillaProcess `
			-EventName ErrorDataReceived -MessageData (Join-Path $RunDir 'vanilla-stderr.log') -Action {
				if ($EventArgs.Data) { Add-Content -LiteralPath $Event.MessageData -Value $EventArgs.Data }
			}
		$script:vanillaProcess.BeginOutputReadLine()
		$script:vanillaProcess.BeginErrorReadLine()
		$script:consoleReady = $false
		$serverReady = $false
		$readyDeadline = (Get-Date).AddSeconds(120)
		while ((Get-Date) -lt $readyDeadline) {
			if ((Test-Path $vanillaLog) -and
				(Select-String -Path $vanillaLog -Pattern 'Done \(' -Quiet -ErrorAction SilentlyContinue)) {
				$serverReady = $true
				break
			}
			if ($script:vanillaProcess.HasExited) { throw "vanilla exited during startup (code $($script:vanillaProcess.ExitCode))" }
			Start-Sleep -Milliseconds 300
		}
		if (-not $serverReady) { throw 'vanilla did not report Done within 120 seconds' }
		$script:consoleReady = $true
	} else {
		# Our server takes 25566 directly (mcproxy's upstream port comes
		# from the vanilla server.properties). The config is generated from
		# the workspace server.toml with only the listen address changed:
		# real values (client jar, protocol version) never enter the repo.
		$oursConfig = Join-Path $RunDir 'server-ours.toml'
		$config = (Get-Content $repoServerToml -Raw) -replace 'listen\s*=\s*"[^"]*"', 'listen = "127.0.0.1:25566"'
		$config = $config -replace "world_dir\s*=\s*'[^']*'", "world_dir = '$($script:activeWorldDir)'"
		if ($OursGameMode -ge 0) {
			if ($config -match '(?m)^game_mode\s*=') {
				$config = $config -replace '(?m)^game_mode\s*=\s*\d+', "game_mode = $OursGameMode"
			} else {
				$config = $config.TrimEnd() + "`r`ngame_mode = $OursGameMode`r`n"
			}
		}
		if ($OursSpawnOverride) {
			if ($config -match '(?m)^spawn_override\s*=') {
				$config = $config -replace '(?m)^spawn_override\s*=\s*"[^"]*"', "spawn_override = `"$OursSpawnOverride`""
			} else {
				$config = $config.TrimEnd() + "`r`nspawn_override = `"$OursSpawnOverride`"`r`n"
			}
		}
		$config | Set-Content $oursConfig -Encoding utf8
		Start-Process -FilePath $serverExe -ArgumentList '--config', $oursConfig `
			-WorkingDirectory (Split-Path -Parent $repoServerToml) `
			-RedirectStandardOutput $oursLog `
			-RedirectStandardError (Join-Path $RunDir 'ours-stderr.log') -PassThru |
			ForEach-Object { $script:oursPid = $_.Id }
	}
	for ($i = 0; $i -lt 120; $i++) {
		if (Get-NetTCPConnection -LocalPort 25566 -State Listen -ErrorAction SilentlyContinue) { break }
		if ($isVanilla -and $script:vanillaProcess.HasExited) { throw "vanilla exited during startup (code $($script:vanillaProcess.ExitCode))" }
		Start-Sleep -Seconds 1
	}
	if (-not (Get-NetTCPConnection -LocalPort 25566 -State Listen -ErrorAction SilentlyContinue)) {
		throw 'the stand server did not open 25566'
	}
	if ($isVanilla) {
		for ($i = 0; $i -lt 30; $i++) {
			if (Get-NetTCPConnection -LocalPort 25575 -State Listen -ErrorAction SilentlyContinue) { break }
			Start-Sleep -Seconds 1
		}
	}
	Start-Process -FilePath $mcproxyExe -ArgumentList '--listen', '127.0.0.1:25565', '--out', $traceDir `
		-RedirectStandardOutput (Join-Path $RunDir 'mcproxy-live.log') `
		-RedirectStandardError (Join-Path $RunDir 'mcproxy-err.log') -PassThru |
		ForEach-Object { $script:mcproxyPid = $_.Id }
	Start-Sleep -Seconds 2
}

function Stop-Stand {
	Stop-ClientByName 'Alice'
	Stop-ClientByName 'Bob'
	Start-Sleep -Seconds 2
	if ($script:vanillaProcess -and -not $script:vanillaProcess.HasExited) {
		try {
			$script:vanillaProcess.StandardInput.WriteLine('stop')
			$script:vanillaProcess.StandardInput.Close()
			if (-not $script:vanillaProcess.WaitForExit(30000)) { $script:vanillaProcess.Kill() }
		} catch { $script:vanillaProcess.Kill() }
	}
	if ($script:mcproxyPid) { Stop-Process -Id $script:mcproxyPid -Force -ErrorAction SilentlyContinue }
	if ($script:vanillaOutputEvent) { Unregister-Event -SourceIdentifier $script:vanillaOutputEvent.Name -ErrorAction SilentlyContinue }
	if ($script:vanillaErrorEvent) { Unregister-Event -SourceIdentifier $script:vanillaErrorEvent.Name -ErrorAction SilentlyContinue }
	if ($script:oursPid) { Stop-Process -Id $script:oursPid -Force -ErrorAction SilentlyContinue }
}

function Invoke-VanillaConsole {
	param([string]$Command)
	if (-not $isVanilla) { throw 'S7 vanilla stdin is unavailable for the selected target' }
	if (-not $script:consoleReady) { throw 'S7 vanilla stdin console is not ready' }
	if ($script:vanillaProcess -and -not $script:vanillaProcess.HasExited) {
		$script:vanillaProcess.StandardInput.WriteLine($Command)
		return $true
	}
	throw 'vanilla stdin is not available; no fallback was used without explicit per-run confirmation'
}

# RCON is retained for existing non-S7 scenarios only; S7 prefers stdin.
function Invoke-StandRcon {
	param([string]$Command)
	if ($isVanilla) { return (& $rconScript -Command $Command -ServerHost '127.0.0.1' -Password 'test') }
}

# Launches one test client and waits for its fresh join line in the log,
# then gives the client time to finish world load (the "joined" server line
# races the client's own player_loaded: commands sent right after the join
# line were silently not sent on the wire - measured on trace, player_loaded
# lands ~3.5 s after connect, so 5 s of margin closes the race).
# Returns { pid, uuid }. $Since guards against stale "joined" lines.
function Start-Client {
	param([string]$Nick, [int]$ControlPort, [datetime]$Since, [int]$TimeoutSec = 120)
	$out = & $launchScript -Nick $Nick -Address '127.0.0.1:25565' -ControlPort $ControlPort
	$pidOut = [int]($out | Select-Object -Last 1)
	$uuid = $null
	foreach ($line in $out) {
		if ($line -match 'offline uuid ([0-9a-f-]+)') { $uuid = $Matches[1] }
	}
	if ($isVanilla) {
		if (-not (Wait-FreshJoin -Log $vanillaLog -Nick $Nick -Since $Since -TimeoutSec $TimeoutSec)) {
			throw "$Nick did not join in ${TimeoutSec}s"
		}
	} else {
		# Our server logs its join differently and has no RCON: the join
		# gate is the mod's own self-observe (inGame=true), the same gate
		# the vanilla stand uses its log for.
		$deadline = (Get-Date).AddSeconds($TimeoutSec)
		$inGame = $false
		while ((Get-Date) -lt $deadline) {
			try {
				$resp = Invoke-TestCmd $ControlPort (New-ObserveCmd 'join-gate') 3000
				if ($resp.ok -and $resp.result.inGame) { $inGame = $true; break }
			} catch { }
			Start-Sleep -Milliseconds 300
		}
		if (-not $inGame) { throw "$Nick did not reach inGame in ${TimeoutSec}s" }
	}
	Start-Sleep -Seconds 5
	return [PSCustomObject]@{ pid = $pidOut; uuid = $uuid }
}

# Stops a client and returns the moment to use as a fresh-line watermark.
function Stop-Client {
	param([string]$Nick)
	Stop-ClientByName $Nick
	Start-Sleep -Seconds 2
	return Get-Date
}

# --- S1: pose persistence across relaunch (owner flow) ---------------------
function Invoke-S1 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$tol = [float]$Cfg.rotationToleranceDeg

	# A joins, turns (a movement tap makes the client flush the angles to
	# the server - the wire carries them in move_player_rot / player_input),
	# disconnects; the turn must land in playerdata.
	$watermark = Get-Date
	$launchA = Start-Client $nickA $portA $watermark
	$null = Invoke-TestCmd $portA (New-LookCmd 's1-turn' $Cfg.turn.dx $Cfg.turn.dy)
	$null = Invoke-TestCmd $portA (New-KeyCmd 's1-tap' 'up' 'tap')
	Start-Sleep -Milliseconds 500
	$saved = (Invoke-TestCmd $portA (New-ObserveCmd 's1-saved')).result.rot
	$null = Invoke-TestCmd $portA (New-SessionCmd 's1-leave' 'disconnect')
	Start-Sleep -Seconds 2
	# The owner flow saves through RCON; our server has no RCON (stage G),
	# so the save step is a recorded gap there - the world state is simply
	# whatever the server keeps in memory at disconnect.
	Invoke-StandRcon 'save-all flush'
	$watermark = Stop-Client $nickA

	# A relaunches (fresh process, quickPlay) and only then B launches.
	$relaunchAt = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB $relaunchAt

	# Check 1: within 500 ms of B's entry B sees A with non-zero yaw/pitch.
	$limit = [int]$Cfg.bCheckLimitMs
	$poll = [int]$Cfg.bCheckPollMs
	$seen = $null
	$deadline = (Get-Date).AddMilliseconds($limit)
	while ((Get-Date) -lt $deadline) {
		$seen = (Invoke-TestCmd $portB (New-ObserveCmd 's1-obs' $nickA)).result
		if ($seen.found -and $seen.rot.yaw -ne 0 -and $seen.rot.pitch -ne 0) { break }
		Start-Sleep -Milliseconds $poll
	}
	$nonZero = ($seen -and $seen.found -and $seen.rot.yaw -ne 0 -and $seen.rot.pitch -ne 0)
	Add-Check $Result 'B sees non-zero yaw/pitch of A within 500 ms of entry' $nonZero `
		("observed yaw={0} pitch={1} found={2} (saved was yaw={3} pitch={4})" -f `
			$seen.rot.yaw, $seen.rot.pitch, $seen.found, $saved.yaw, $saved.pitch)

	# Check 2: the turn survived into playerdata (owner: NBT of the world
	# copy OR the relaunch trace). Primary: the playerdata NBT - the
	# authoritative saved state. Secondary: the relaunch trace carries a
	# clientbound rotation packet with the saved angles.
	$null = Invoke-TestCmd $portA (New-SessionCmd 's1-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's1-offb' 'disconnect')
	Start-Sleep -Seconds 3
	Invoke-StandRcon 'save-all flush'
	Start-Sleep -Seconds 1
	$nbtPath = if ($isVanilla) { Get-PlayerDataPath (Join-Path $vanillaDir 'world') $launchA.uuid } else { $null }
	$rot = $null
	if ($nbtPath -and (Test-Path $nbtPath)) { $rot = Get-PlayerRotationFromNbt $nbtPath }
	$nbtHit = ($rot -and [Math]::Abs($rot.yaw - [float]$saved.yaw) -le $tol -and [Math]::Abs($rot.pitch - [float]$saved.pitch) -le $tol)
	$newTraces = Get-NewTraceFiles $traceDir $relaunchAt
	$traceHit = $false
	foreach ($trace in $newTraces) {
		if (Find-RotationInTrace (Read-Trace $trace.FullName) ([float]$saved.yaw) ([float]$saved.pitch) $tol) { $traceHit = $true }
	}
	# The owner condition is "NBT OR the relaunch trace": either is enough.
	# On our server there is no playerdata at all (it never writes one), so
	# the fact says so explicitly rather than looking like a parse miss.
	$nbtFact = if ($rot) {
		"yaw=$($rot.yaw) pitch=$($rot.pitch)"
	} elseif ($isVanilla) {
		'no NBT file'
	} else {
		'no playerdata (our server does not write one)'
	}
	Add-Check $Result 'saved rotation present in playerdata (NBT or relaunch trace)' ($nbtHit -or $traceHit) `
		("NBT {0}: {1} -> {2}; relaunch trace -> {3} (saved yaw={4} pitch={5})" -f `
			$nbtPath, $nbtFact, $(if ($nbtHit) { 'hit' } else { 'miss' }), `
			$(if ($traceHit) { 'hit' } else { 'miss' }), $saved.yaw, $saved.pitch)
}

# --- S1b: observer joins while A is already turned (stage 10, encoder A) ----
# A enters, turns (west, down), taps movement and stands still; only then B
# enters. Check: B sees A with non-zero yaw/pitch within 500 ms of B's entry.
# The vanilla reference gives the join-packet bytes (add_entity angles, head
# yaw, packet order); our run is expected red until encoder A lands.
function Invoke-S1b {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB

	$watermark = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Invoke-TestCmd $portA (New-LookCmd 's1b-turn' $Cfg.turn.dx $Cfg.turn.dy)
	$null = Invoke-TestCmd $portA (New-KeyCmd 's1b-tap' 'up' 'tap')
	Start-Sleep -Milliseconds 500
	$posed = (Invoke-TestCmd $portA (New-ObserveCmd 's1b-posed')).result.rot
	# A stands still from here on; B joins and looks at A at once.
	$null = Start-Client $nickB $portB (Get-Date).AddSeconds(-5)
	$limit = [int]$Cfg.bCheckLimitMs
	$poll = [int]$Cfg.bCheckPollMs
	$seen = $null
	$deadline = (Get-Date).AddMilliseconds($limit)
	while ((Get-Date) -lt $deadline) {
		$seen = (Invoke-TestCmd $portB (New-ObserveCmd 's1b-obs' $nickA)).result
		if ($seen.found -and $seen.rot.yaw -ne 0 -and $seen.rot.pitch -ne 0) { break }
		Start-Sleep -Milliseconds $poll
	}
	$nonZero = ($seen -and $seen.found -and $seen.rot.yaw -ne 0 -and $seen.rot.pitch -ne 0)
	Add-Check $Result 'B sees non-zero yaw/pitch of A within 500 ms of entry' $nonZero `
		("observed yaw={0} pitch={1} found={2} (A posed yaw={3} pitch={4})" -f `
			$seen.rot.yaw, $seen.rot.pitch, $seen.found, $posed.yaw, $posed.pitch)
	$null = Invoke-TestCmd $portA (New-SessionCmd 's1b-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's1b-offb' 'disconnect')
	Start-Sleep -Seconds 3
}

# --- S2: angle deltas reach the server and the other client ----------------
function Invoke-S2 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$degPerDelta = [float]$Cfg.degPerDelta
	$dTol = [float]$Cfg.deltaToleranceDeg

	$watermark = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB $watermark

	# Reset pitch to ~0 first: the persisted pose may look straight down and
	# a dy delta would then clamp at +-90 deg, breaking the expected value.
	$reset = (Invoke-TestCmd $portA (New-ObserveCmd 's2-reset')).result.rot
	$resetDy = -[double]$reset.pitch / $degPerDelta
	if ([Math]::Abs($resetDy) -gt 0.5) {
		$null = Invoke-TestCmd $portA (New-LookCmd 's2-reset-look' 0 ([Math]::Round($resetDy)))
		Start-Sleep -Milliseconds 300
	}
	foreach ($turn in $Cfg.turns) {
		$before = (Invoke-TestCmd $portB (New-ObserveCmd 's2-pre' $nickA)).result.rot
		$null = Invoke-TestCmd $portA (New-LookCmd 's2-turn' $turn.dx $turn.dy)
		# Rotation alone never reaches the server; a movement tap makes the
		# client send the new angles inside player_input.
		$null = Invoke-TestCmd $portA (New-KeyCmd 's2-tap' 'up' 'tap')
		Start-Sleep -Milliseconds 800
		$after = (Invoke-TestCmd $portB (New-ObserveCmd 's2-post' $nickA)).result.rot
		$expYaw = [float]$before.yaw + [float]$turn.dx * $degPerDelta
		$expPitch = [float]$before.pitch + [float]$turn.dy * $degPerDelta
		$dYaw = [Math]::Abs([float]$after.yaw - $expYaw)
		$dPitch = [Math]::Abs([float]$after.pitch - $expPitch)
		Add-Check $Result ("turn dx={0} dy={1}: yaw delta seen by B" -f $turn.dx, $turn.dy) ($dYaw -le $dTol) `
			("yaw before={0} after={1} expected={2} (delta {3} deg)" -f $before.yaw, $after.yaw, $expYaw, $dYaw)
		Add-Check $Result ("turn dx={0} dy={1}: pitch delta seen by B" -f $turn.dx, $turn.dy) ($dPitch -le $dTol) `
			("pitch before={0} after={1} expected={2} (delta {3} deg)" -f $before.pitch, $after.pitch, $expPitch, $dPitch)
	}
	$null = Invoke-TestCmd $portA (New-SessionCmd 's2-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's2-offb' 'disconnect')
	Start-Sleep -Seconds 3
}

# --- S3: air swing vs hold-on-block (fact and order by seq) ----------------
function Invoke-S3 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$airTaps = [int]$Cfg.airTaps
	$gapMs = [int]$Cfg.airGapMs
	$digMs = [int]$Cfg.digHoldMs

	$watermark = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB $watermark

	# Wait for the observer's initial chunk stream before generating events.
	# Otherwise the test may disconnect B while it is still receiving chunks,
	# truncating the proxy trace before the swing broadcasts are recorded.
	$expectedChunks = 473
	$loadedChunks = 0
	$chunkDeadline = (Get-Date).AddSeconds(120)
	while ((Get-Date) -lt $chunkDeadline) {
		$observation = Invoke-TestCmd $portB (New-ObserveCmd 's3-chunk-load' $nickA)
		if ($observation.ok -and $observation.result.loadedChunks) {
			$loadedChunks = [int]$observation.result.loadedChunks
			if ($loadedChunks -ge $expectedChunks) { break }
		}
		Start-Sleep -Milliseconds 500
	}
	Add-Check $Result 'B finishes initial chunk loading before S3' ($loadedChunks -ge $expectedChunks) `
		("loaded chunks={0}; expected at least {1}" -f $loadedChunks, $expectedChunks)

	# Air phase: look straight up (crosshair can only hit sky) and tap
	# attack - a swing in air must be punch (46) without player_action (41).
	$null = Invoke-TestCmd $portA (New-LookCmd 's3-up' 0 -4000)
	for ($i = 0; $i -lt $airTaps; $i++) {
		$null = Invoke-TestCmd $portA (New-KeyCmd 's3-air' 'attack')
		Start-Sleep -Milliseconds $gapMs
	}

	# Dig phase: look straight down (at the block underfoot) and hold
	# attack - holding on a block keeps punching and adds player_action.
	# Stage 11: on our server the client does not see blocks yet (S6), so
	# the dig phase is not run there - recorded as a fact, never red.
	if ($isVanilla) {
		$null = Invoke-TestCmd $portA (New-LookCmd 's3-down' 0 4000)
		Start-Sleep -Milliseconds 300
		$null = Invoke-TestCmd $portA (New-KeyCmd 's3-dig-on' 'attack' 'down')
		Start-Sleep -Milliseconds $digMs
		$null = Invoke-TestCmd $portA (New-KeyCmd 's3-dig-off' 'attack' 'up')
		Start-Sleep -Milliseconds 500
	} else {
		Add-Check $Result 'dig phase: skipped on ours (client does not see blocks, S6)' $true `
			'not run; only the air segment is the S3 acceptance for ours'
	}

	$null = Invoke-TestCmd $portA (New-SessionCmd 's3-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's3-offb' 'disconnect')
	Start-Sleep -Seconds 3

	# Parse A's trace: punch (46) and player_action (41) seq numbers.
	# seq is per connection, so serverbound order is read from A's trace
	# only (first new file) and the swing broadcast from B's (second).
	# Air segment = everything before the first player_action; dig segment =
	# the rest. Counts are never asserted - only presence and order.
	$traces = Get-NewTraceFiles $traceDir $watermark
	$aTrace = $traces | Select-Object -First 1
	$bTrace = $traces | Select-Object -Skip 1 | Select-Object -First 1
	$punchSeqs = @()
	$actionSeqs = @()
	if ($aTrace) {
		$rows = Read-Trace $aTrace.FullName
		$punchSeqs = Get-TraceSeqs $rows 'punch'
		$actionSeqs = Get-TraceSeqs $rows 'player_action'
	}
	$firstAction = if ($actionSeqs.Count) { ($actionSeqs | Measure-Object -Minimum).Minimum } else { [int]::MaxValue }
	$airPunches = @($punchSeqs | Where-Object { $_ -lt $firstAction })
	Add-Check $Result 'air swings: punch packets with no player_action before them' `
		($airPunches.Count -ge 1) `
		("punch seqs before first action: [{0}]; action seqs: [{1}]" -f ($airPunches -join ','), ($actionSeqs -join ','))
	if ($isVanilla) {
		$digPunches = @($punchSeqs | Where-Object { $_ -gt $firstAction })
		Add-Check $Result 'hold on block: player_action present and punch continues after it' `
			($actionSeqs.Count -ge 1 -and $digPunches.Count -ge 1) `
			("action seqs: [{0}]; punch seqs after first action: [{1}]" -f ($actionSeqs -join ','), ($digPunches -join ','))
	}

	# A->B visibility: the swing broadcast (clientbound swing_animation)
	# reaches B; A's own trace never carries it (broadcast excludes self).
	# Stage 11: the air segment must carry exactly one swing per tap, and
	# every payload must be the traced bytes: VarInt entity id + 00 01 06.
	# The air segment of B ends at the first block_destruction (dig phase
	# output); a trace without destructions is all air.
	$bSwingRows = @()
	$firstDestructionSeq = [int]::MaxValue
	$payloadOk = $true
	$payloadFact = ''
	if ($bTrace) {
		$bRows = Read-Trace $bTrace.FullName
		foreach ($row in $bRows) {
			if ($row.dir -ne 'clientbound') { continue }
			if ($row.name -eq 'minecraft:block_destruction') {
				if ([int]$row.seq -lt $firstDestructionSeq) { $firstDestructionSeq = [int]$row.seq }
				continue
			}
			if ($row.name -match 'swing_animation|animate') {
				$bSwingRows += $row
				$payload = ConvertFrom-TraceB64 $row.b64
				# Decode the leading VarInt entity id, then expect 00 01 06.
				$varInt = ConvertFrom-TraceVarInt $payload 0
				$offset = $varInt.next
				$tail = if ($payload.Length -eq $offset + 3) {
					@($payload[$offset], $payload[$offset + 1], $payload[$offset + 2])
				} else { @() }
				$rowHex = [BitConverter]::ToString($payload).Replace('-', ' ').ToLowerInvariant()
				if (-not ($tail.Count -eq 3 -and $tail[0] -eq 0 -and $tail[1] -eq 1 -and $tail[2] -eq 6)) {
					$payloadOk = $false
					$payloadFact = "$payloadFact ; seq $($row.seq): $rowHex"
				}
			}
		}
	}
	$airSwingRows = @($bSwingRows | Where-Object { [int]$_.seq -lt $firstDestructionSeq })
	Add-Check $Result 'B receives swing broadcast while A swings' ($airSwingRows.Count -ge 1) `
		("clientbound swing_animation/animate rows in B's trace ({0}): {1}" -f $bTrace.Name, $airSwingRows.Count)
	Add-Check $Result 'air swings reach B exactly once per tap' `
		($airSwingRows.Count -eq $airTaps) `
		("air swing rows before first block_destruction (seq {0}): {1}, taps {2}" -f `
			$(if ($firstDestructionSeq -ne [int]::MaxValue) { $firstDestructionSeq } else { 'none' }), `
			$airSwingRows.Count, $airTaps)
	Add-Check $Result 'every swing payload is VarInt id + 00 01 06' $payloadOk `
		$(if ($payloadFact) { "mismatching payloads:$payloadFact" } else { "all $($bSwingRows.Count) payloads match the traced bytes" })
}

# Reads the observer's actual remote-player pose. Retry one time so a snapshot
# taken during the entity's initial client-side load does not become a false negative.
function Invoke-RemotePoseObservation {
	param($Port, $Player, $ExpectedPose, [bool]$ExpectedCrouching, [object]$ExpectedSprinting)
	$response = $null
	$observed = $null
	$pass = $false
	$attempts = 0
	for ($attempt = 1; $attempt -le 2; $attempt++) {
		$attempts = $attempt
		$script:s5ObservationIndex++
		$observeId = "pose-observe-$($script:s5ObservationIndex)-$attempt"
		$response = Invoke-TestCmd $Port (New-ObserveCmd $observeId $Player)
		$observed = if ($response.ok) { $response.result } else { $null }
		$pass = $observed -and $observed.found -eq $true -and $observed.pose -eq $ExpectedPose -and $observed.isCrouching -eq $ExpectedCrouching
		# The sprint bit is only asserted when the caller passes a value;
		# older traces carry no isSprinting field at all.
		if ($null -ne $ExpectedSprinting) {
			$pass = $pass -and $observed.isSprinting -eq [bool]$ExpectedSprinting
		}
		if ($pass -or $attempt -eq 2) { break }
		Start-Sleep -Milliseconds 250
	}
	$fact = 'attempts={0}; found={1} pose={2} isCrouching={3} isSprinting={4}; expected pose={5} isCrouching={6} isSprinting={7}' -f `
		$attempts, `
		$(if ($observed) { $observed.found } else { $false }), `
		$(if ($observed) { $observed.pose } else { 'unavailable' }), `
		$(if ($observed) { $observed.isCrouching } else { 'unavailable' }), `
		$(if ($observed -and $observed.PSObject.Properties['isSprinting']) { $observed.isSprinting } else { 'unavailable' }), `
		$ExpectedPose, $ExpectedCrouching, `
		$(if ($null -ne $ExpectedSprinting) { $ExpectedSprinting } else { 'unasserted' })
	return [PSCustomObject]@{ pass=[bool]$pass; fact=$fact }
}

# Picks the sprint attribute record that follows a given metadata flag row.
#
# The two S5 attribute checks are independent facts: a missing release flag must
# not hide a sprint-on payload that is present in the trace. Run
# 2026-11-15_1908 carried `01 01 1a ... minecraft:sprinting ...` at seq 849 and
# still reported "missing" because the selection was gated on the full four-flag
# sequence matching. `AfterSeq` is the anchor flag row; `$null` means the anchor
# was never captured, which is reported as such instead of as a missing payload.
function Select-SprintAttribute {
	param($Attributes, [object]$AfterSeq, [bool]$Sprint)
	if ($null -eq $AfterSeq) {
		return [PSCustomObject]@{ found = $false; seq = $null; hex = $null; reason = 'anchor metadata flag was not captured' }
	}
	$hit = $Attributes | Where-Object {
		$_.seq -gt [int]$AfterSeq -and ([bool]$_.sprint -eq $Sprint)
	} | Select-Object -First 1
	if (-not $hit) {
		$kind = if ($Sprint) { 'sprint-on' } else { 'sprint-off' }
		return [PSCustomObject]@{ found = $false; seq = $null; hex = $null; reason = "no $kind attribute record after seq $AfterSeq" }
	}
	return [PSCustomObject]@{ found = $true; seq = $hit.seq; hex = $hit.hex; reason = 'ok' }
}

# Summarizes the observer timeline captured after the sprint release.
#
# The sample at +0 ms is a timeline fact, not a deadline: the server applies the
# release on its next tick and the packet needs one more hop, so the observer
# may legitimately still report isSprinting=true there (run
# 2026-11-15_s5-live-clean: true at +8 ms, false at +159 ms). The check passes
# when a later sample reports the sprint-off; the whole timeline is kept as the
# fact so a slow transition stays visible in the report.
function Get-SprintOffObservation {
	param($Observations)
	$timeline = @()
	$observed = $false
	foreach ($observation in $Observations) {
		$timeline += ("+{0}/{1}ms={2}/{3}" -f $observation.targetAfterReleaseMs, $observation.actualAfterReleaseMs, $observation.found, $observation.isSprinting)
		if ($observation.found -and $observation.isSprinting -eq $false) { $observed = $true }
	}
	return [PSCustomObject]@{ observed = $observed; timeline = ($timeline -join '; ') }
}

# Waits until the observer's loaded-chunk count stops changing. The client keeps
# draining the join batch for seconds after `inGame`, and entity packets for A
# are queued behind that drain on the same connection (run 2026-11-15_1908:
# 473 chunks still pending when the scenario disconnected). Facts sampled before
# the drain settles describe the drain, not the pose transition.
# Falls back to `StableSamples` reads with a bounded wait so a client that never
# settles still produces a fact instead of hanging the scenario.
function Wait-ObserverChunksStable {
	param([int]$Port, [int]$TimeoutSec = 30)
	$deadline = (Get-Date).AddSeconds($TimeoutSec)
	$previous = -1
	$stable = 0
	$reads = 0
	$last = -1
	while ((Get-Date) -lt $deadline) {
		$reads++
		try {
			$response = Invoke-TestCmd $Port (New-ObserveCmd 'chunk-gate') 5000
			if ($response.ok -and $response.result.PSObject.Properties['loadedChunks']) {
				$last = [int]$response.result.loadedChunks
			}
		} catch { }
		if ($last -eq $previous) { $stable++ } else { $stable = 0 }
		$previous = $last
		if ($stable -ge 2) { break }
		Start-Sleep -Milliseconds 120
	}
	return [PSCustomObject]@{
		settled = ($stable -ge 2)
		loadedChunks = $last
		reads = $reads
	}
}

# --- S5: sneak and sprint flags seen by B (stage 10, encoder B reference) ---
# A holds shift / sprint, then releases. Facts come from the traces: the
# serverbound player_input flags in A's trace vs the clientbound
# set_entity_data rows for A's entity id in B's trace (index, type, value).
# S5 also checks the actual remote-player pose on B via observe.
function Invoke-S7 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB

	foreach ($pose in $Cfg.poses) {
		$watermark = Get-Date
		$existingTraceNames = @(Get-ChildItem $traceDir -Filter 'proxy-conn-*.jsonl' -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
		$joinBAt = $null
		$launchA = Start-Client $nickA $portA $watermark
		if (-not $launchA.uuid) { throw "$($pose.name): could not resolve Alice's offline UUID" }
		$spawnX = [double]$Cfg.safeSpawn.x
		$spawnY = [double]$Cfg.safeSpawn.y
		$spawnZ = [double]$Cfg.safeSpawn.z
		$tpCommand = "tp $nickA $spawnX $spawnY $spawnZ"
		if ($isVanilla) {
			Invoke-VanillaConsole $tpCommand | Out-Null
			$script:s7Facts.Add("$($pose.name): teleport command sent via vanilla stdin: $tpCommand")
		} else {
			$script:s7Facts.Add("$($pose.name): ours target uses configured spawn; no teleport command is available")
		}
		# A fixed delay is not proof of teleport application: keep observing until
		# the client reports the exact destination, then enforce at least 2 s there.
		# If stdin was accepted but the client never moves, retry once over loopback
		# RCON; this stand's stdin command was logged without a matching teleport packet.
		$ready = $null
		$readyError = $null
		$positionError = [double]::PositiveInfinity
		$rconFallbackAt = (Get-Date).AddSeconds(2)
		$rconFallbackUsed = $false
		$tpDeadline = (Get-Date).AddSeconds(15)
		while ((Get-Date) -lt $tpDeadline) {
			$readyResponse = Invoke-TestCmd $portA (New-ObserveCmd ('s7-' + $pose.name + '-safe-spawn'))
			if (-not $readyResponse.ok -or -not $readyResponse.result.inGame) { throw "$($pose.name): Alice observe unavailable after teleport" }
			$ready = $readyResponse.result
			$targetPos = $ready.pos
			$positionError = [Math]::Sqrt([Math]::Pow($targetPos.x - $spawnX, 2) + [Math]::Pow($targetPos.y - $spawnY, 2) + [Math]::Pow($targetPos.z - $spawnZ, 2))
			if ($positionError -le 0.1) { break }
			$readyError = "client has not reached target; pos=($($targetPos.x),$($targetPos.y),$($targetPos.z)) error=$positionError"
			if (-not $isVanilla) { break }
			if (-not $rconFallbackUsed -and (Get-Date) -ge $rconFallbackAt) {
				$rconResult = Invoke-StandRcon $tpCommand
				$rconFallbackUsed = $true
				$script:s7Facts.Add("$($pose.name): stdin did not move Alice; loopback RCON fallback sent '$tpCommand'; response='$rconResult'")
			}
			Start-Sleep -Milliseconds 250
		}
		$atTarget = $ready -and $positionError -le 0.1
		if ($atTarget) {
			Start-Sleep -Seconds 2
			$readyResponse = Invoke-TestCmd $portA (New-ObserveCmd ('s7-' + $pose.name + '-safe-spawn-confirm'))
			if (-not $readyResponse.ok -or -not $readyResponse.result.inGame) { throw "$($pose.name): Alice observe unavailable after teleport wait" }
			$ready = $readyResponse.result
			$targetPos = $ready.pos
			$positionError = [Math]::Sqrt([Math]::Pow($targetPos.x - $spawnX, 2) + [Math]::Pow($targetPos.y - $spawnY, 2) + [Math]::Pow($targetPos.z - $spawnZ, 2))
			$atTarget = $positionError -le 0.1
			if (-not $atTarget) { $readyError = "client left target during 2 s wait; pos=($($targetPos.x),$($targetPos.y),$($targetPos.z)) error=$positionError" }
		} elseif (-not $isVanilla) {
			$atTarget = $true
			$readyError = "using actual server spawn pos=($($ready.pos.x),$($ready.pos.y),$($ready.pos.z)); distance from vanilla safeSpawn=$positionError"
		}
		if (-not $atTarget) {
			$script:s7Facts.Add("$($pose.name): teleport fact: stdin command='$tpCommand'; client did not confirm target and 2 s stable wait; $readyError")
			Add-Check $Result ("$($pose.name): teleport target confirmed by Alice") $false $readyError
			Add-Check $Result ("$($pose.name): held-input and observer phases skipped") $false 'teleport was not confirmed; no inputs were sent and B was not launched'
			Stop-ClientByName $nickA
			Start-Sleep -Seconds 2
			continue
		}
		$feetSafe = $ready.feetBlock.air -or $ready.feetBlock.collisionShapeEmpty
		$headSafe = $ready.headBlock.air -or $ready.headBlock.collisionShapeEmpty
		$supportSafe = $ready.supportBlock.collisionShapeFullBlock
		$grounded = ($ready.onGround -eq $true) -and $supportSafe -and $feetSafe -and $headSafe
		$spawnFact = "stdin command='$tpCommand'; observe pos=($($targetPos.x),$($targetPos.y),$($targetPos.z)) onGround=$($ready.onGround); feet=$($ready.feetBlock.id) head=$($ready.headBlock.id) support=$($ready.supportBlock.id); safe=$grounded"
		if ($ready.horizontalCollision) { $script:s7Facts.Add("$($pose.name): post-tp horizontalCollision=true before input") }
		$script:s7Facts.Add("$($pose.name): teleport fact: $spawnFact; waited at least 2 s after client position confirmation")
		Add-Check $Result ("$($pose.name): Alice reached safe spawn and is on ground") $grounded $spawnFact
		if (-not $grounded) {
			$script:s7Facts.Add("$($pose.name): skipped held input and B join because the runtime spawn check failed")
			Add-Check $Result ("$($pose.name): held-input and observer phases skipped") $false 'unsafe spawn; no inputs were sent and B was not launched'
			Stop-ClientByName $nickA
			Start-Sleep -Seconds 2
			continue
		}

		$null = Invoke-TestCmd $portA (New-KeyCmd ('s7-' + $pose.name + '-on') $pose.key 'down')
		if ($pose.alsoKey) {
			$null = Invoke-TestCmd $portA (New-KeyCmd ('s7-' + $pose.name + '-also-on') $pose.alsoKey 'down')
		}
		Start-Sleep -Milliseconds ([int]$pose.holdMs)
		$heldResponse = Invoke-TestCmd $portA (New-ObserveCmd ('s7-' + $pose.name + '-held'))
		if (-not $heldResponse.ok -or -not $heldResponse.result.inGame) { throw "$($pose.name): held-state observe unavailable" }
		$held = $heldResponse.result
		$heldInput = if ($pose.key -eq 'shift') { $held.keyShiftDown } else { $held.keySprintDown -and $held.keyUpDown -and $held.sprinting }
		$notBlocked = if ($pose.key -eq 'sprint') { -not $held.horizontalCollision } else { $true }
		Add-Check $Result ("$($pose.name): held keys confirmed before B entry") ($heldInput -and $notBlocked) `
			("shift={0} up={1} sprintKey={2} sprinting={3} horizontalCollision={4}" -f $held.keyShiftDown, $held.keyUpDown, $held.keySprintDown, $held.sprinting, $held.horizontalCollision)
		if (-not ($heldInput -and $notBlocked)) {
			$script:s7Facts.Add("$($pose.name): skipped B join because held input or clear sprint path was not confirmed")
			Add-Check $Result ("$($pose.name): observer phase skipped") $false 'held-state runtime check failed; B was not launched'
			Stop-ClientByName $nickA
			Start-Sleep -Seconds 2
			continue
		}

		$joinBAt = Get-Date
		$null = Start-Client $nickB $portB $joinBAt
		# Restrict B facts to its post-login play stream; earlier config packets are irrelevant.
		$expectedPose = if ($pose.key -eq 'shift') { 'CROUCHING' } else { 'STANDING' }
		$expectedCrouching = ($pose.key -eq 'shift')
		$poseObservation = Invoke-RemotePoseObservation $portB $nickA $expectedPose $expectedCrouching
		$script:s7Facts.Add("$($pose.name): B client observe: $($poseObservation.fact)")
		Add-Check $Result ("$($pose.name): B client observes expected pose") $poseObservation.pass $poseObservation.fact
		Start-Sleep -Milliseconds 500
		$null = Invoke-TestCmd $portA (New-SessionCmd ('s7-' + $pose.name + '-off') 'disconnect')
		$null = Invoke-TestCmd $portB (New-SessionCmd ('s7-' + $pose.name + '-offb') 'disconnect')
		Start-Sleep -Seconds 3

		$traces = @(Get-ChildItem $traceDir -Filter 'proxy-conn-*.jsonl' -ErrorAction SilentlyContinue |
			Where-Object { $existingTraceNames -notcontains $_.Name } | Sort-Object CreationTime)
		$aTrace = $null
		$bTrace = $null
		if ($traces.Count -ge 1) { $aTrace = $traces[0] }
		if ($traces.Count -ge 2) { $bTrace = $traces[1] }
		if (-not $aTrace -or -not $bTrace) {
			Add-Check $Result ("$($pose.name): A/B traces captured") $false ("A={0}; B={1}; B watermark={2:o}" -f $aTrace, $bTrace, $joinBAt)
			continue
		}
		$aRows = Read-Trace $aTrace.FullName
		$bAllRows = Read-Trace $bTrace.FullName
		$bLogin = $bAllRows | Where-Object { $_.state -eq 'play' -and $_.dir -eq 'clientbound' -and $_.name -eq 'minecraft:login' } | Select-Object -First 1
		if (-not $bLogin) {
			Add-Check $Result ("$($pose.name): B login/play boundary found") $false 'minecraft:login not found in B trace'
			continue
		}
		$bRows = @($bAllRows | Where-Object { $_.seq -gt $bLogin.seq -and $_.state -eq 'play' -and $_.dir -eq 'clientbound' })
		$inputRows = @($aRows | Where-Object { $_.dir -eq 'serverbound' -and $_.name -eq 'minecraft:player_input' } |
			ForEach-Object { $payload = ConvertFrom-TraceB64 $_.b64; if ($payload.Length) { '{0:x2}' -f $payload[0] } })
		$inputSeen = if ($pose.key -eq 'shift') { $inputRows -contains '20' } else { $inputRows -contains '41' }
		Add-Check $Result ("$($pose.name): A trace contains held player_input flag") $inputSeen `
			("player_input flags: {0}" -f ($inputRows -join ' '))

		$uuidBytes = ConvertTo-NetworkUuidBytes ([guid]::Parse($launchA.uuid))
		$spawnMatch = Find-EntitySpawn $bRows $uuidBytes
		if (-not $spawnMatch) {
			Add-Check $Result ("$($pose.name): B add_entity for A captured") $false 'spawn UUID not found in B post-login play trace'
			continue
		}
		$spawn = $spawnMatch.row
		$entityId = [int]$spawnMatch.entityId

		$stateRows = @($bRows | Where-Object {
			if ($_.dir -ne 'clientbound' -or $_.state -ne 'play' -or $_.seq -le $bLogin.seq -or $_.name -ne 'minecraft:set_entity_data') { return $false }
			$p = ConvertFrom-TraceB64 $_.b64
			if ($p.Length -lt 2) { return $false }
			(ConvertFrom-TraceVarInt $p 0).value -eq $entityId
		} | ForEach-Object { $p = ConvertFrom-TraceB64 $_.b64; '{0}:{1}' -f $_.seq, ([BitConverter]::ToString($p).Replace('-', ' ').ToLowerInvariant()) })
		$attributeRows = @($bRows | Where-Object {
			if ($_.dir -ne 'clientbound' -or $_.state -ne 'play' -or $_.seq -le $bLogin.seq -or $_.name -ne 'minecraft:update_attributes') { return $false }
			$p = ConvertFrom-TraceB64 $_.b64
			if ($p.Length -lt 2) { return $false }
			$entity = ConvertFrom-TraceVarInt $p 0
			$count = ConvertFrom-TraceVarInt $p $entity.next
			$entity.value -eq $entityId -and $count.value -gt 0 -and [Text.Encoding]::ASCII.GetString($p).Contains('minecraft:sprinting')
		} | ForEach-Object { $p = ConvertFrom-TraceB64 $_.b64; '{0}:{1}' -f $_.seq, ([BitConverter]::ToString($p).Replace('-', ' ').ToLowerInvariant()) })
		$spawnBytes = [BitConverter]::ToString((ConvertFrom-TraceB64 $spawn.b64)).Replace('-', ' ').ToLowerInvariant()
		Add-Check $Result ("$($pose.name): B add_entity spawn payload recorded") $true "entity=$entityId seq=$($spawn.seq) $spawnBytes"
		$fact = "B connection=$($bTrace.Name), login play records=$($bRows.Count), A spawn seq=$($spawn.seq) bytes=$spawnBytes; metadata seq/hex=[$($stateRows -join '; ')]; attribute seq/hex=[$($attributeRows -join '; ')]"
		$script:s7Facts.Add("$($pose.name): $fact")
		Add-Check $Result ("$($pose.name): B state packets recorded without inference") $true $fact
		Stop-ClientByName $nickA
		Stop-ClientByName $nickB
		Start-Sleep -Seconds 2
	}
}

function Invoke-S5 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB

	$watermark = Get-Date
	$launchA = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB $watermark
	$script:s5ObservationIndex = 0
	$script:s5PoseChecks = [System.Collections.ArrayList]::new()
	$script:s5ReleaseObservations = [System.Collections.ArrayList]::new()
	# Stage 11: B keeps receiving the join chunk batch for several seconds after
	# it reports inGame, and the server's entity stream to B competes with that
	# drain (run 2026-11-15_1908: 473 chunks still in flight when the scenario
	# disconnected). Pose facts sampled during the drain describe the drain, not
	# the transition. Wait until B's loaded-chunk count stops moving.
	$script:s5ObserverReadiness = Wait-ObserverChunksStable $portB 30
	foreach ($pose in $Cfg.poses) {
		if ($pose.alsoKey) {
			$null = Invoke-TestCmd $portA (New-KeyCmd ('s5-' + $pose.name + '-also-on') $pose.alsoKey 'down')
		}
		$null = Invoke-TestCmd $portA (New-KeyCmd ('s5-' + $pose.name + '-on') $pose.key 'down')
		Start-Sleep -Milliseconds ([int]$pose.holdMs)
		$expectedPose = if ($pose.key -eq 'shift') { 'CROUCHING' } else { 'STANDING' }
		$expectedCrouching = ($pose.key -eq 'shift')
		# Stage 11: the observer's sprint bit is asserted as well - the
		# sprint pose must report isSprinting=true, the sneak one false.
		$expectedSprinting = if ($pose.key -eq 'shift') { $false } else { $true }
		$poseObservation = Invoke-RemotePoseObservation $portB $nickA $expectedPose $expectedCrouching $expectedSprinting
		$script:s5PoseChecks.Add([PSCustomObject]@{ name=$pose.name; pass=$poseObservation.pass; fact=$poseObservation.fact }) | Out-Null
		$null = Invoke-TestCmd $portA (New-KeyCmd ('s5-' + $pose.name + '-off') $pose.key 'up')
		if ($pose.alsoKey) {
			$null = Invoke-TestCmd $portA (New-KeyCmd ('s5-' + $pose.name + '-also-off') $pose.alsoKey 'up')
		}
		if ($pose.key -eq 'sprint') {
			$releaseAt = Get-Date
			$previousSampleMs = 0
			# Stage 11: the sprint-off transition needs the server tick plus the
			# delivery slot, and run 2026-11-15_1908 showed ~0.5 s of tick
			# lateness on top of that. Sampling stops as soon as the observer
			# reports sprinting=false, so a healthy run still ends within ~0 ms;
			# the later samples only bound a slow one.
			foreach ($sampleAfterReleaseMs in @(0, 100, 300, 700, 1500, 3000, 5000)) {
				$waitMs = $sampleAfterReleaseMs - $previousSampleMs
				if ($waitMs -gt 0) { Start-Sleep -Milliseconds $waitMs }
				$releaseResponse = Invoke-TestCmd $portB (New-ObserveCmd "s5-sprint-release-$sampleAfterReleaseMs" $nickA)
				$releaseObservation = if ($releaseResponse.ok) { $releaseResponse.result } else { $null }
				$observedAt = Get-Date
				$foundRelease = [bool]($releaseObservation -and $releaseObservation.found)
				$sprintingAfterRelease = if ($releaseObservation -and $releaseObservation.PSObject.Properties['isSprinting']) { $releaseObservation.isSprinting } else { $null }
				$script:s5ReleaseObservations.Add([PSCustomObject]@{
					targetAfterReleaseMs = $sampleAfterReleaseMs
					actualAfterReleaseMs = [Math]::Round(($observedAt - $releaseAt).TotalMilliseconds, 1)
					observedAt = $observedAt.ToString('o')
					found = $foundRelease
					isSprinting = $sprintingAfterRelease
					pose = if ($releaseObservation) { $releaseObservation.pose } else { $null }
					isCrouching = if ($releaseObservation) { $releaseObservation.isCrouching } else { $null }
				}) | Out-Null
				$previousSampleMs = $sampleAfterReleaseMs
				if ($foundRelease -and $sprintingAfterRelease -eq $false) { break }
			}
		}
		Start-Sleep -Milliseconds 800
	}
	$null = Invoke-TestCmd $portA (New-SessionCmd 's5-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's5-offb' 'disconnect')
	Start-Sleep -Seconds 3

	$traces = Get-NewTraceFiles $traceDir $watermark
	$aTrace = $traces | Select-Object -First 1
	$bTrace = $traces | Select-Object -Skip 1 | Select-Object -First 1
	if (-not $aTrace -or -not $bTrace) {
		Add-Check $Result 'S5 has A and B traces' $false ("A={0}; B={1}" -f $aTrace, $bTrace)
		foreach ($poseCheck in $script:s5PoseChecks) {
			Add-Check $Result ("S5 $($poseCheck.name): B observe reports expected pose") $poseCheck.pass $poseCheck.fact
		}
		Add-Check $Result 'S5 observer chunk drain settled before pose sampling' $script:s5ObserverReadiness.settled `
			("loadedChunks={0}; reads={1}; settled={2}" -f $script:s5ObserverReadiness.loadedChunks, $script:s5ObserverReadiness.reads, $script:s5ObserverReadiness.settled)
		return
	}
	$aRows = Read-Trace $aTrace.FullName
	$bRows = Read-Trace $bTrace.FullName
	$inputFlags = @($aRows | Where-Object { $_.dir -eq 'serverbound' -and $_.name -eq 'minecraft:player_input' } |
		ForEach-Object { $payload = ConvertFrom-TraceB64 $_.b64; if ($payload.Length) { [int]$payload[0] } })
	$expectedInputs = @(0x20, 0x00, 0x01, 0x41, 0x01, 0x00)
	$inputMatch = $inputFlags.Count -ge $expectedInputs.Count
	if ($inputMatch) {
		for ($i = 0; $i -lt $expectedInputs.Count; $i++) {
			if ($inputFlags[$i] -ne $expectedInputs[$i]) { $inputMatch = $false; break }
		}
	}
	Add-Check $Result 'A sends trace-confirmed player_input flags 20 00 01 41 01 00' $inputMatch `
		("player_input flags: {0}" -f (($inputFlags | ForEach-Object { '{0:x2}' -f $_ }) -join ' '))

	# Resolve A's entity id from the AddEntity UUID in B's own trace so the
	# scenario also works against our server, whose runtime ids differ.
	$uuidBytes = ConvertTo-NetworkUuidBytes ([guid]::Parse($launchA.uuid))
	$spawnMatch = Find-EntitySpawn $bRows $uuidBytes
	if (-not $spawnMatch) {
		Add-Check $Result 'S5 resolves A entity id from B add_entity' $false 'spawn UUID not found in B trace'
		return
	}
	$spawn = $spawnMatch.row
	$entityId = [int]$spawnMatch.entityId

	$metadata = @($bRows | Where-Object {
		if ($_.dir -ne 'clientbound' -or $_.name -ne 'minecraft:set_entity_data') { return $false }
		$p = ConvertFrom-TraceB64 $_.b64
		$p.Length -eq 5 -and $p[0] -eq $entityId -and $p[1] -eq 0 -and $p[2] -eq 0 -and $p[4] -eq 0xff
	} | ForEach-Object { $p = ConvertFrom-TraceB64 $_.b64; [PSCustomObject]@{ seq=[int]$_.seq; flags=[int]$p[3]; hex=[BitConverter]::ToString($p).Replace('-', ' ').ToLowerInvariant() } })
	$expectedFlags = @(0x02, 0x00, 0x08, 0x00)
	$found = @()
	$cursor = 0
	foreach ($expected in $expectedFlags) {
		$match = $null
		for ($i = $cursor; $i -lt $metadata.Count; $i++) {
			if ($metadata[$i].flags -eq $expected) { $match = $metadata[$i]; $cursor = $i + 1; break }
		}
		$found += ,$match
	}
	$stateMatch = $found.Count -eq 4 -and @($found | Where-Object { $null -eq $_ }).Count -eq 0
	Add-Check $Result 'B receives exact metadata flags 02 00 08 00 in order' $stateMatch `
		("entity={0}; seq/hex: {1}" -f $entityId, (($found | ForEach-Object { if ($_){ "$($_.seq)/$($_.hex)" } }) -join ', '))
	$releaseRow = if ($found.Count -ge 4) { $found[3] } else { $null }
	if ($null -ne $releaseRow) {
		$windowStart = [Math]::Max(0, $releaseRow.seq - 30)
		$windowEnd = [Math]::Min($bRows.Count - 1, $releaseRow.seq + 30)
		$releaseWindow = @($bRows | Where-Object { [int]$_.seq -ge $windowStart -and [int]$_.seq -le $windowEnd })
		$entityWindowRows = @($releaseWindow | Where-Object {
			if ($_.dir -ne 'clientbound' -or $_.name -notin @('minecraft:set_entity_data', 'minecraft:update_attributes')) { return $false }
			$p = ConvertFrom-TraceB64 $_.b64
			$p.Length -gt 0 -and [int]$p[0] -eq $entityId
		}) | ForEach-Object {
			$payload = ConvertFrom-TraceB64 $_.b64
			[PSCustomObject]@{ seq=[int]$_.seq; read_ms=$_.read_ms; name=$_.name; hex=[BitConverter]::ToString($payload).Replace('-', ' ').ToLowerInvariant() }
		}
		$windowFact = ($entityWindowRows | ForEach-Object { "{0}@{1} {2} {3}" -f $_.seq, $_.read_ms, $_.name, $_.hex }) -join '; '
		$Result | Add-Member -NotePropertyName sprintOffTrace -NotePropertyValue ([PSCustomObject]@{
			entityId = $entityId; releaseSeq = $releaseRow.seq; windowStart = $windowStart; windowEnd = $windowEnd; packets = $entityWindowRows
		}) -Force
		Add-Check $Result 'S5 sprint-off B trace window (+/-30 packets around release)' ($entityWindowRows.Count -gt 0) `
			("entity={0}; window={1}..{2}; {3}" -f $entityId, $windowStart, $windowEnd, $windowFact)
	} else {
		Add-Check $Result 'S5 sprint-off B trace window (+/-30 packets around release)' $false 'sprint-off metadata was absent; no release anchor to center the requested trace window'
	}

	$poseRows = @($bRows | Where-Object {
		if ($_.dir -ne 'clientbound' -or $_.name -ne 'minecraft:set_entity_data') { return $false }
		$p = ConvertFrom-TraceB64 $_.b64
		$p.Length -eq 5 -and $p[0] -eq $entityId -and $p[1] -eq 0x06 -and $p[2] -eq 0x14 -and $p[4] -eq 0xff
	} | ForEach-Object { $p = ConvertFrom-TraceB64 $_.b64; [PSCustomObject]@{seq=[int]$_.seq; pose=[int]$p[3]; hex=[BitConverter]::ToString($p).Replace('-', ' ').ToLowerInvariant()} })
	$expectedPoses = @(0x05, 0x00)
	$poseFound = @()
	$poseCursor = 0
	foreach ($expected in $expectedPoses) {
		$match = $null
		for ($i = $poseCursor; $i -lt $poseRows.Count; $i++) {
			if ($poseRows[$i].pose -eq $expected) { $match = $poseRows[$i]; $poseCursor = $i + 1; break }
		}
		$poseFound += ,$match
	}
	$poseMatch = $poseFound.Count -eq 2 -and @($poseFound | Where-Object { $null -eq $_ }).Count -eq 0
	Add-Check $Result 'B receives separate sneak poses 05 then 00' $poseMatch `
		("entity={0}; seq/hex: {1}" -f $entityId, (($poseFound | ForEach-Object { if ($_){ "$($_.seq)/$($_.hex)" } }) -join ', '))

	foreach ($poseCheck in $script:s5PoseChecks) {
		Add-Check $Result ("S5 $($poseCheck.name): B observe reports expected pose") $poseCheck.pass $poseCheck.fact
	}
	Add-Check $Result 'S5 observer chunk drain settled before pose sampling' $script:s5ObserverReadiness.settled `
		("loadedChunks={0}; reads={1}; settled={2}" -f $script:s5ObserverReadiness.loadedChunks, $script:s5ObserverReadiness.reads, $script:s5ObserverReadiness.settled)
	$sprintOffObservation = Get-SprintOffObservation $script:s5ReleaseObservations
	$Result | Add-Member -NotePropertyName sprintOffObservations -NotePropertyValue $script:s5ReleaseObservations -Force
	Add-Check $Result 'S5 B observe sees sprint-off after release (the +0 ms sample is a timeline fact, not a deadline)' `
		$sprintOffObservation.observed ("timeline: {0}" -f $sprintOffObservation.timeline)

	$attributes = @($bRows | Where-Object {
		if ($_.dir -ne 'clientbound' -or $_.name -ne 'minecraft:update_attributes') { return $false }
		$p = ConvertFrom-TraceB64 $_.b64
		$p.Length -ge 3 -and $p[0] -eq $entityId -and $p[1] -eq 1 -and $p[2] -eq 0x1a
	} | ForEach-Object { $p = ConvertFrom-TraceB64 $_.b64; [PSCustomObject]@{seq=[int]$_.seq; hex=[BitConverter]::ToString($p).Replace('-', ' ').ToLowerInvariant(); sprint=([Text.Encoding]::ASCII.GetString($p).Contains('minecraft:sprinting'))} })
	$onSelection = Select-SprintAttribute $attributes $(if ($found[2]) { $found[2].seq } else { $null }) $true
	$offSelection = Select-SprintAttribute $attributes $(if ($found[3]) { $found[3].seq } else { $null }) $false
	$attributeSuffix = if ($entityId -lt 0x80) { '{0:x2}' -f $entityId } else { $null }
	$onHex = $attributeSuffix + ' 01 1a 3f b9 99 99 a0 00 00 00 01 13 6d 69 6e 65 63 72 61 66 74 3a 73 70 72 69 6e 74 69 6e 67 3f d3 33 33 40 00 00 00 02'
	$offHex = $attributeSuffix + ' 01 1a 3f b9 99 99 a0 00 00 00 00'
	$onPass = $onSelection.found -and $onSelection.hex -eq $onHex
	$offPass = $offSelection.found -and $offSelection.hex -eq $offHex
	$onDetail = if ($onSelection.found) { "seq=$($onSelection.seq) hex=$($onSelection.hex)" } else { "missing: $($onSelection.reason)" }
	$offDetail = if ($offSelection.found) { "seq=$($offSelection.seq) hex=$($offSelection.hex)" } else { "missing: $($offSelection.reason)" }
	Add-Check $Result 'Sprint-on attribute payload follows flags 08' $onPass $onDetail
	Add-Check $Result 'Sprint-off attribute payload follows flags 00' $offPass $offDetail
}

# --- S6: dig probe (stage 10, subtask 0d, facts only) ----------------------
# A looks down, snapshots observe (hitType, gameMode, block under crosshair),
# holds attack, snapshots again. The facts explain why no player_action (41)
# is sent on our server: crosshair miss, wrong game mode or cooldown. No check
# asserts a dig outcome.
function Invoke-S6 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$nickA = [string]$Cfg.playerA

	$watermark = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Invoke-TestCmd $portA (New-LookCmd 's6-look' $Cfg.lookDown.dx $Cfg.lookDown.dy)
	$null = Invoke-TestCmd $portA (New-KeyCmd 's6-tap' 'up' 'tap')
	Start-Sleep -Milliseconds 500
	$before = (Invoke-TestCmd $portA (New-ObserveCmd 's6-before')).result
	$null = Invoke-TestCmd $portA (New-KeyCmd 's6-attack-on' 'attack' 'down')
	Start-Sleep -Milliseconds ([int]$Cfg.attackHoldMs)
	$during = (Invoke-TestCmd $portA (New-ObserveCmd 's6-during')).result
	$null = Invoke-TestCmd $portA (New-KeyCmd 's6-attack-off' 'attack' 'up')
	$null = Invoke-TestCmd $portA (New-SessionCmd 's6-off' 'disconnect')
	Start-Sleep -Seconds 3

	$fmt = {
		param($o)
		'hitType={0} gameMode={1} block=({2},{3},{4}) crosshairEntity={5}' -f `
			$o.hitType, $o.gameMode, $o.block.x, $o.block.y, $o.block.z, $o.crosshairEntity
	}
	Add-Check $Result 'dig probe facts captured (before attack)' ($null -ne $before) (& $fmt $before)
	Add-Check $Result 'dig probe facts captured (during attack)' ($null -ne $during) (& $fmt $during)
}

# --- S4: game mode changes are visible to the other client -----------------
function Invoke-S4 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$settle = [int]$Cfg.settleMs

	$watermark = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB $watermark

	$initial = (Invoke-TestCmd $portB (New-ObserveCmd 's4-pre' $nickA)).result
	Add-Check $Result 'B sees A in survival before changes' ($initial.gameMode -eq 'survival') `
		("gameMode seen by B: {0}" -f $initial.gameMode)
	foreach ($mode in $Cfg.modes) {
		# Game mode changes reach A through the console command; our server
		# has no RCON (stage G), so the change is skipped there and the
		# check records "no RCON" as the fact instead of a fake pass.
		if ($isVanilla) {
			Invoke-StandRcon ("gamemode {0} {1}" -f $mode, $nickA)
			Start-Sleep -Milliseconds $settle
			$seen = (Invoke-TestCmd $portB (New-ObserveCmd 's4-obs' $nickA)).result
			Add-Check $Result ("B sees A in {0} after RCON gamemode" -f $mode) ($seen.gameMode -eq $mode) `
				("gameMode seen by B: {0} (expected {1})" -f $seen.gameMode, $mode)
		} else {
			Add-Check $Result ("B sees A in {0} after gamemode change" -f $mode) $false `
				'no RCON on our server - the gamemode change path is not wired (owner: S4 vs ours is a recorded gap)'
		}
	}
	$null = Invoke-TestCmd $portA (New-SessionCmd 's4-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's4-offb' 'disconnect')
	Start-Sleep -Seconds 3
}

# --- S8: water (runbook: water y 37..41 at x 105..107, z -32..-28) ----------
# A sprints on land, is teleported by RCON into the water column, swims,
# releases and is teleported back to the start. B is the observer; its
# clientbound trace is the data for docs/SWIMMING.md: flags/pose metadata for
# A, update_attributes (water_movement_efficiency?), level_event/sound around
# the entry. Vanilla only (the teleport is an RCON command).
function Invoke-S8 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$settle = [int]$Cfg.settleMs
	$inv = [System.Globalization.CultureInfo]::InvariantCulture

	$watermark = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB $watermark
	# Stage 11: B keeps draining the join chunk batch for seconds; pose facts
	# sampled during the drain describe the drain, not the player (S5 note).
	$observer = Wait-ObserverChunksStable $portB 30
	Write-Host ("  [info] B chunk drain settled={0} loadedChunks={1} reads={2}" -f `
			$observer.settled, $observer.loadedChunks, $observer.reads)
	$script:s7Facts.Add(("S8 B chunk drain: settled={0} loadedChunks={1}" -f `
			$observer.settled, $observer.loadedChunks))

	if (-not $isVanilla) {
		# Ours (Шаг 14d): no RCON, so the water run uses the dev-only spawn
		# override. -OursSpawnOverride puts the joining player above open water
		# (config only, the save and the template stay untouched); A falls in
		# and B observes the swimming metadata our server computes from the
		# shared ChunkStore.
		if (-not $OursSpawnOverride) {
			Add-Check $Result 'water entry needs -OursSpawnOverride on our server' $false `
				'no RCON on our server and no -OursSpawnOverride was set - the water run is a vanilla-trace fact'
			$null = Invoke-TestCmd $portA (New-SessionCmd 's8-off' 'disconnect')
			$null = Invoke-TestCmd $portB (New-SessionCmd 's8-offb' 'disconnect')
			Start-Sleep -Seconds 3
			return
		}
		# Let A (and B, both spawned at the override) fall from the air into
		# the water column before sampling.
		Start-Sleep -Milliseconds ([int]$Cfg.fallMs)
		$wet = (Invoke-TestCmd $portA (New-ObserveCmd 's8-wet')).result
		Add-Check $Result 'A is in water after falling from the spawn override' `
			($null -ne $wet -and ($wet.feetBlock.id -eq 'minecraft:water' -or $wet.headBlock.id -eq 'minecraft:water')) `
			("feet={0} head={1} pose={2} onGround={3} y={4}" -f `
				$wet.feetBlock.id, $wet.headBlock.id, $wet.pose, $wet.onGround, $wet.pos.y)

		# At rest in water the Шаг 14e rule says A is NOT swimming: the bit
		# follows `feet_in_water && sprint`, so a still player reports the
		# standing pose with no 0x10 bit. This is the check the old 14d run
		# got wrong (it expected SWIMMING at rest).
		$rest = (Invoke-TestCmd $portB (New-ObserveCmd 's8-rest' $nickA)).result
		Add-Check $Result 'B sees A NOT swimming at rest in water (sprint required)' `
			($null -ne $rest -and $rest.pose -ne 'SWIMMING' -and -not $rest.isSwimming) `
			("pose={0} isSwimming={1} isSprinting={2} pos=({3},{4},{5})" -f `
				$rest.pose, $rest.isSwimming, $rest.isSprinting, $rest.pos.x, $rest.pos.y, $rest.pos.z)

		# Now A holds forward + sprint through KeyMapping; B must see A swimming.
		$null = Invoke-TestCmd $portA (New-KeyCmd 's8-swim-forward' 'up' 'down')
		$null = Invoke-TestCmd $portA (New-KeyCmd 's8-swim-sprint' 'sprint' 'down')
		Start-Sleep -Milliseconds ([int]$Cfg.swimMs)
		$swim = (Invoke-TestCmd $portB (New-ObserveCmd 's8-swim' $nickA)).result
		Add-Check $Result 'B sees A swimming (pose SWIMMING) when sprinting in water' `
			($swim.pose -eq 'SWIMMING' -and $swim.isSwimming) `
			("pose={0} isSwimming={1} isSprinting={2} pos=({3},{4},{5})" -f `
				$swim.pose, $swim.isSwimming, $swim.isSprinting, $swim.pos.x, $swim.pos.y, $swim.pos.z)
		$null = Invoke-TestCmd $portA (New-KeyCmd 's8-swim-forward-off' 'up' 'up')
		$null = Invoke-TestCmd $portA (New-KeyCmd 's8-swim-sprint-off' 'sprint' 'up')

		$script:s7Facts.Add(("S8 ours in water: feet={0} head={1} pose={2} y={3}" -f `
				$wet.feetBlock.id, $wet.headBlock.id, $wet.pose, $wet.pos.y))
		$script:s7Facts.Add(("S8 ours observed by B at rest: pose={0} isSwimming={1}" -f $rest.pose, $rest.isSwimming))
		$script:s7Facts.Add(("S8 ours observed by B sprinting: pose={0} isSwimming={1}" -f $swim.pose, $swim.isSwimming))

		$null = Invoke-TestCmd $portA (New-SessionCmd 's8-off' 'disconnect')
		$null = Invoke-TestCmd $portB (New-SessionCmd 's8-offb' 'disconnect')
		Start-Sleep -Seconds 3
		return
	}

	$start = (Invoke-TestCmd $portA (New-ObserveCmd 's8-start')).result
	Add-Check $Result 'A is on dry land before the water run' `
		($null -ne $start -and $start.inGame -and $start.onGround) `
		("pos=({0},{1},{2}) feet={3}" -f $start.pos.x, $start.pos.y, $start.pos.z, $start.feetBlock.id)

	# 1) Sprint on land: the known S5 baseline, kept in the same trace so the
	# water packets can be read against it.
	$null = Invoke-TestCmd $portA (New-KeyCmd 's8-forward-on' 'up' 'down')
	$null = Invoke-TestCmd $portA (New-KeyCmd 's8-sprint-on' 'sprint' 'down')
	Start-Sleep -Milliseconds ([int]$Cfg.sprintMs)
	$land = (Invoke-TestCmd $portB (New-ObserveCmd 's8-land' $nickA)).result
	Add-Check $Result 'B sees A sprinting on land' ($land.isSprinting -eq $true) `
		("pose={0} isSprinting={1} pos=({2},{3},{4})" -f `
			$land.pose, $land.isSprinting, $land.pos.x, $land.pos.y, $land.pos.z)
	$null = Invoke-TestCmd $portA (New-KeyCmd 's8-forward-off' 'up' 'up')
	$null = Invoke-TestCmd $portA (New-KeyCmd 's8-sprint-off' 'sprint' 'up')

	# 2) Into the water column, at rest first (the entry observation).
	$wx = ([double]$Cfg.water.x).ToString($inv)
	$wy = ([double]$Cfg.water.y).ToString($inv)
	$wz = ([double]$Cfg.water.z).ToString($inv)
	Invoke-StandRcon ("tp {0} {1} {2} {3}" -f $nickA, $wx, $wy, $wz) | Out-Null
	Start-Sleep -Milliseconds $settle
	$wet = (Invoke-TestCmd $portA (New-ObserveCmd 's8-wet')).result
	Add-Check $Result 'A is in water after the teleport' `
		($wet.feetBlock.id -eq 'minecraft:water' -or $wet.headBlock.id -eq 'minecraft:water') `
		("feet={0} head={1} pose={2} onGround={3}" -f `
			$wet.feetBlock.id, $wet.headBlock.id, $wet.pose, $wet.onGround)

	# 3) Swim: forward + sprint held, the pose the swimmer reports.
	$null = Invoke-TestCmd $portA (New-KeyCmd 's8-swim-forward' 'up' 'down')
	$null = Invoke-TestCmd $portA (New-KeyCmd 's8-swim-sprint' 'sprint' 'down')
	Start-Sleep -Milliseconds ([int]$Cfg.swimMs)
	$swim = (Invoke-TestCmd $portB (New-ObserveCmd 's8-swim' $nickA)).result
	Add-Check $Result 'B sees A swimming (pose SWIMMING)' ($swim.pose -eq 'SWIMMING') `
		("pose={0} isSprinting={1} pos=({2},{3},{4})" -f `
			$swim.pose, $swim.isSprinting, $swim.pos.x, $swim.pos.y, $swim.pos.z)
	$null = Invoke-TestCmd $portA (New-KeyCmd 's8-swim-forward-off' 'up' 'up')
	$null = Invoke-TestCmd $portA (New-KeyCmd 's8-swim-sprint-off' 'sprint' 'up')
	Start-Sleep -Milliseconds $settle

	# 4) Out of the water: back to the recorded start position.
	$sx = ([double]$start.pos.x).ToString($inv)
	$sy = ([double]$start.pos.y).ToString($inv)
	$sz = ([double]$start.pos.z).ToString($inv)
	Invoke-StandRcon ("tp {0} {1} {2} {3}" -f $nickA, $sx, $sy, $sz) | Out-Null
	Start-Sleep -Milliseconds $settle
	$dry = (Invoke-TestCmd $portB (New-ObserveCmd 's8-dry' $nickA)).result
	Add-Check $Result 'B sees A out of the water after the exit teleport' ($dry.pose -ne 'SWIMMING') `
		("pose={0} pos=({1},{2},{3})" -f $dry.pose, $dry.pos.x, $dry.pos.y, $dry.pos.z)

	$script:s7Facts.Add(("S8 land baseline: pose={0} isSprinting={1}" -f $land.pose, $land.isSprinting))
	$script:s7Facts.Add(("S8 in water (at rest): feet={0} head={1} pose={2}" -f `
			$wet.feetBlock.id, $wet.headBlock.id, $wet.pose))
	$script:s7Facts.Add(("S8 swimming: pose={0} isSprinting={1} pos=({2},{3},{4})" -f `
			$swim.pose, $swim.isSprinting, $swim.pos.x, $swim.pos.y, $swim.pos.z))
	$script:s7Facts.Add(("S8 after exit: pose={0}" -f $dry.pose))

	$null = Invoke-TestCmd $portA (New-SessionCmd 's8-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's8-offb' 'disconnect')
	Start-Sleep -Seconds 3
}

# --- S9: creative instant block break (both traces kept) --------------------
# A is put into creative by RCON, looks down so the crosshair sits on a block,
# taps a movement key to flush the angles, then attacks. Vanilla facts from
# the 2026-11-16 runs: a tap (setDown true then false) sends no packets at
# all, a held click sends player_action(start) + punch per targeted block and
# the server answers each with block_update(air) + block_changed_ack - no
# abort/stop action while the hold lasts. At the spawn pad the crosshair
# reaches a dark-oak-leaf dip beside the pad (the ray is not vertical), so A
# does not fall; the check is that the crosshair lost its target, not a y
# drop. The A trace carries the serverbound side, the B trace the observer
# side: level_event(2001) with the destroyed state id, swing_animation, and
# block_update - no block_destruction and no sound in creative.
function Invoke-S9 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$settle = [int]$Cfg.settleMs

	$watermark = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB $watermark
	$observer = Wait-ObserverChunksStable $portB 30
	Write-Host ("  [info] B chunk drain settled={0} loadedChunks={1} reads={2}" -f `
			$observer.settled, $observer.loadedChunks, $observer.reads)
	$script:s7Facts.Add(("S9 B chunk drain: settled={0} loadedChunks={1}" -f `
			$observer.settled, $observer.loadedChunks))

	if (-not $isVanilla) {
		# Ours (Шаг 14c): creative comes from the generated config
		# (-OursGameMode 1 patches game_mode into server-ours.toml); there is
		# no RCON to switch modes in game. Without it the scenario stays the
		# recorded vanilla-trace gap.
		if ($OursGameMode -lt 0) {
			Add-Check $Result 'creative mode is a vanilla RCON command' $false `
				'no RCON on our server and no -OursGameMode was set - the creative break is a vanilla-trace fact'
			$null = Invoke-TestCmd $portA (New-SessionCmd 's9-off' 'disconnect')
			$null = Invoke-TestCmd $portB (New-SessionCmd 's9-offb' 'disconnect')
			Start-Sleep -Seconds 3
			return
		}
	} else {
		Invoke-StandRcon ("gamemode creative {0}" -f $nickA) | Out-Null
		Start-Sleep -Milliseconds $settle
	}
	$mode = (Invoke-TestCmd $portB (New-ObserveCmd 's9-mode' $nickA)).result
	Add-Check $Result 'B sees A in creative' ($mode.gameMode -eq 'creative') `
		("gameMode seen by B: {0}" -f $mode.gameMode)

	# A downward look (S6 with 150 deltas was a MISS: ~22 degrees never
	# reaches a block) plus a movement tap to flush the angles.
	$look = $Cfg.lookDown
	$null = Invoke-TestCmd $portA (New-LookCmd 's9-look' $look.dx $look.dy)
	$null = Invoke-TestCmd $portA (New-KeyCmd 's9-flush' 'up' 'tap')
	Start-Sleep -Milliseconds $settle
	$before = (Invoke-TestCmd $portA (New-ObserveCmd 's9-before')).result
	Add-Check $Result 'crosshair is on a block before the click' `
		($before.hitType -eq 'BLOCK' -and $null -ne $before.hitBlock) `
		("hitType={0} block=({1},{2},{3}) face={4}" -f `
			$before.hitType, $before.hitBlock.x, $before.hitBlock.y, $before.hitBlock.z, $before.hitBlock.face)

	# Attempt 1: one click tap. Recorded as a fact only - two vanilla runs
	# (2026-11-16_0918, _0932) show the tap sends nothing (no punch, no
	# player_action), so it cannot be asserted per run.
	$null = Invoke-TestCmd $portA (New-KeyCmd 's9-attack-tap' 'attack' 'tap')
	Start-Sleep -Milliseconds $settle
	$afterTap = (Invoke-TestCmd $portA (New-ObserveCmd 's9-after-tap')).result
	$script:s7Facts.Add(("S9 tap: y {0} -> {1} onGround={2}" -f `
			$before.pos.y, $afterTap.pos.y, $afterTap.onGround))

	# Attempt 2: the held click (the proven B3 path). Vanilla destroys each
	# targeted block instantly; afterwards the crosshair must have lost the
	# block it pointed at (a different target or a miss).
	$null = Invoke-TestCmd $portA (New-KeyCmd 's9-attack-down' 'attack' 'down')
	Start-Sleep -Milliseconds ([int]$Cfg.attackHoldMs)
	$null = Invoke-TestCmd $portA (New-KeyCmd 's9-attack-up' 'attack' 'up')
	Start-Sleep -Milliseconds $settle
	$afterHold = (Invoke-TestCmd $portA (New-ObserveCmd 's9-after-hold')).result
	$targetGone = ($afterHold.hitType -ne $before.hitType) -or `
		($null -ne $afterHold.hitBlock -and $null -ne $before.hitBlock -and `
			([int]$afterHold.hitBlock.x -ne [int]$before.hitBlock.x -or `
				[int]$afterHold.hitBlock.y -ne [int]$before.hitBlock.y -or `
				[int]$afterHold.hitBlock.z -ne [int]$before.hitBlock.z))
	Add-Check $Result 'the held attack destroyed the targeted blocks (crosshair target changed)' $targetGone `
		("hitType {0} -> {1}, y {2} -> {3} onGround={4}" -f `
			$before.hitType, $afterHold.hitType, $before.pos.y, $afterHold.pos.y, $afterHold.onGround)
	$witness = (Invoke-TestCmd $portB (New-ObserveCmd 's9-witness' $nickA)).result
	Add-Check $Result 'B observed A after the break' ($witness.found -eq $true) `
		("pose={0} pos=({1},{2},{3})" -f $witness.pose, $witness.pos.x, $witness.pos.y, $witness.pos.z)

	if (-not $isVanilla) {
		# Ours (Шаг 14c): the wire facts of "B sees the block gone" — the
		# break level_event (event 2001, data = the destroyed state id)
		# reaches B before the air block_update, the S3/S9 observer order.
		$bFiles = Get-NewTraceFiles $traceDir $watermark
		$bTrace = $bFiles | Select-Object -Skip 1 | Select-Object -First 1
		$breakOk = $false
		$breakFacts = 'no B trace'
		if ($bTrace) {
			# The traces are tens of megabytes: prefilter the raw lines with
			# Select-String and parse only the block-break rows (a full
			# Read-Trace of 57 MB takes tens of minutes in PowerShell).
			$bRows = @(Select-String -LiteralPath $bTrace.FullName -Pattern 'minecraft:(level_event|block_update)' |
					ForEach-Object { $_.Line | ConvertFrom-Json } |
					Where-Object { $_.dir -eq 'clientbound' -and $_.state -eq 'play' })
			$breakLevels = @($bRows | Where-Object {
					if ($_.name -ne 'minecraft:level_event') { return $false }
					$p = ConvertFrom-TraceB64 $_.b64
					$p.Length -ge 4 -and $p[0] -eq 0x00 -and $p[1] -eq 0x00 -and $p[2] -eq 0x07 -and $p[3] -eq 0xd1
				})
			$airUpdates = @($bRows | Where-Object {
					if ($_.name -ne 'minecraft:block_update') { return $false }
					$p = ConvertFrom-TraceB64 $_.b64
					$p.Length -eq 9 -and $p[8] -eq 0
				})
			$levelSeqs = @($breakLevels | ForEach-Object { [int]$_.seq })
			$airSeqs = @($airUpdates | ForEach-Object { [int]$_.seq })
			# One level_event per broken block, each followed by its air
			# update: check the pairs (max<min fails on an interleaved pair).
			$breakOk = ($levelSeqs.Count -ge 1) -and ($levelSeqs.Count -eq $airSeqs.Count)
			if ($breakOk) {
				for ($i = 0; $i -lt $levelSeqs.Count; $i++) {
					if ($levelSeqs[$i] -ge $airSeqs[$i]) { $breakOk = $false; break }
				}
			}
			$breakFacts = "level_event(2001)={0} block_update(air)={1} ordered={2}" -f `
				$breakLevels.Count, $airUpdates.Count, $breakOk
		}
		Add-Check $Result 'B receives the break level_event 2001 before the air block_update' $breakOk $breakFacts
	}

	$script:s7Facts.Add(("S9 creative crosshair: hitType={0} block=({1},{2},{3})" -f `
			$before.hitType, $before.hitBlock.x, $before.hitBlock.y, $before.hitBlock.z))
	$script:s7Facts.Add(("S9 after the hold: hitType={0} y {1} -> {2}" -f `
			$afterHold.hitType, $before.pos.y, $afterHold.pos.y))

	$null = Invoke-TestCmd $portA (New-SessionCmd 's9-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's9-offb' 'disconnect')
	Start-Sleep -Seconds 3
}

# --- S10: swimming/crawling decision data (Шаг 14e) -------------------------
# Builds small geometries in the air above spawn and replays a fixed input
# program in each, sampling A (self labels) and B (remote labels) as reference
# facts. The byte-level truth stays in the mcproxy traces; the samples and the
# exact build commands go to <runDir>\s10-samples.json. This function only
# contributes facts: the decision table is derived from the traces, not here.

# Builds the console commands for one geometry and the A stand point.
function Get-S10Build {
	param($Geo, [int]$BaseY, [int]$X, [int]$Z)
	$cx = $X
	$cz = $Z
	$commands = New-Object System.Collections.Generic.List[string]
	$standX = [double]$cx + 0.5
	$standY = [double]($BaseY + 1)
	$standZ = [double]$cz + 0.5
	$yaw = 0.0
	# The y of the "enter from above" teleport: above the water surface, never
	# inside it (a 4-deep pool otherwise starts the fall already submerged).
	$dropY = $standY
	switch ([string]$Geo.kind) {
		'pool' {
			$d = [int]$Geo.depth
			# 9x9 stone platform, one-block stone walls, 7x7 water interior.
			$commands.Add("fill $($cx - 4) $BaseY $($cz - 4) $($cx + 4) $BaseY $($cz + 4) minecraft:stone")
			$commands.Add("fill $($cx - 4) $($BaseY + 1) $($cz - 4) $($cx + 4) $($BaseY + $d) $($cz + 4) minecraft:stone")
			$commands.Add("fill $($cx - 3) $($BaseY + 1) $($cz - 3) $($cx + 3) $($BaseY + $d) $($cz + 3) minecraft:water")
			$dropY = [double]($BaseY + $d + 2)
		}
		'waterlogged' {
			$commands.Add("fill $($cx - 3) $BaseY $($cz - 3) $($cx + 3) $BaseY $($cz + 3) minecraft:stone")
			$level = $BaseY
			if ($Geo.at -eq 'feet') { $level = $BaseY + 1 }
			$commands.Add("fill $($cx - 1) $level $($cz - 1) $($cx + 1) $level $($cz + 1) $([string]$Geo.block)")
		}
		'plant' {
			$commands.Add("fill $($cx - 2) $BaseY $($cz - 2) $($cx + 2) $BaseY $($cz + 2) minecraft:sand")
			$commands.Add("fill $($cx - 2) $($BaseY + 1) $($cz - 2) $($cx + 2) $($BaseY + 2) $($cz + 2) minecraft:stone")
			$commands.Add("fill $($cx - 1) $($BaseY + 1) $($cz - 1) $($cx + 1) $($BaseY + 2) $($cz + 1) minecraft:water")
			$commands.Add("setblock $cx $($BaseY + 1) $cz $([string]$Geo.block)")
			if ($Geo.block2) { $commands.Add("setblock $cx $($BaseY + 2) $cz $([string]$Geo.block2)") }
		}
		'tunnel' {
			# A long walkway with side walls, an open approach (no roof) and a
			# roofed corridor the player walks into: x >= cx-2 is covered, so the
			# player must lower the pose on the way in. The roof block decides
			# the clearance: full stone = 1 block, top slab = 1.5 blocks.
			$commands.Add("fill $($cx - 5) $BaseY $($cz - 1) $($cx + 3) $BaseY $($cz + 1) minecraft:stone")
			$commands.Add("fill $($cx - 5) $($BaseY + 1) $($cz - 1) $($cx + 3) $($BaseY + 1) $($cz - 1) minecraft:stone")
			$commands.Add("fill $($cx - 5) $($BaseY + 1) $($cz + 1) $($cx + 3) $($BaseY + 1) $($cz + 1) minecraft:stone")
			$commands.Add("fill $($cx - 2) $($BaseY + 2) $($cz - 1) $($cx + 3) $($BaseY + 2) $($cz + 1) $([string]$Geo.roof)")
			$commands.Add("fill $($cx + 3) $($BaseY + 1) $($cz - 1) $($cx + 3) $($BaseY + 2) $($cz + 1) minecraft:stone")
			$standX = [double]($cx - 4) + 0.5
			$yaw = -90.0
		}
		default { throw "unknown S10 geometry kind '$($Geo.kind)'" }
	}
	return [PSCustomObject]@{
		name         = [string]$Geo.name
		kind         = [string]$Geo.kind
		program      = [string]$Geo.program
		enterFromTop = [bool]$Geo.enterFromTop
		dropY        = [double]$dropY
		commands     = $commands
		stand        = [PSCustomObject]@{ x = $standX; y = $standY; z = $standZ; yaw = $yaw; pitch = 0.0 }
	}
}

function Invoke-S10 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$baseY = [int]$Cfg.baseY
	$xs = @($Cfg.geometryRow.xs)
	$zs = @($Cfg.geometryRow.zs)
	$sampleEvery = [int]$Cfg.sampleEveryMs
	$inv = [System.Globalization.CultureInfo]::InvariantCulture
	$script:s10Samples = New-Object System.Collections.ArrayList
	$script:s10SampleIndex = 0

	# Commands are generated once and kept in the artifact so the world edit is
	# reproducible without reading this function.
	$geometries = New-Object System.Collections.ArrayList
	foreach ($geo in $Cfg.geometries) {
		$slot = [int]$geo.slot
		$col = $slot % $xs.Count
		$row = [int][Math]::Floor($slot / $xs.Count)
		$build = Get-S10Build $geo $baseY $xs[$col] $zs[$row]
		$geometries.Add($build) | Out-Null
	}
	$allCommands = New-Object System.Collections.ArrayList
	foreach ($cmd in @($Cfg.platform)) { $allCommands.Add([string]$cmd) | Out-Null }
	foreach ($geo in $geometries) { foreach ($cmd in $geo.commands) { $allCommands.Add([string]$cmd) | Out-Null } }

	$watermark = Get-Date
	$launchA = Start-Client $nickA $portA $watermark
	if (-not $launchA.uuid) { throw 'S10: could not resolve Alice offline UUID' }
	$null = Start-Client $nickB $portB (Get-Date)
	$observer = Wait-ObserverChunksStable $portB 30
	$script:s7Facts.Add(("S10 B chunk drain: settled={0} loadedChunks={1}" -f $observer.settled, $observer.loadedChunks))

	if (-not $isVanilla) {
		# Our server has no /fill: the geometry must be pre-baked into the run
		# world before start. Recorded as an honest gap, like S4/S9.
		Add-Check $Result 'S10 geometry needs a pre-baked world on our server' $false `
			'no /fill on our server; bake the s10-samples.json commands into the run world first and use -OursSpawnOverride (see docs/SWIMMING.md)'
		$null = Invoke-TestCmd $portA (New-SessionCmd 's10-off' 'disconnect')
		$null = Invoke-TestCmd $portB (New-SessionCmd 's10-offb' 'disconnect')
		Start-Sleep -Seconds 3
		return
	}

	# The build runs only after a client is connected: before that the server
	# answers every /fill with "That position is not loaded" (first s10 run,
	# 2026-11-16_1500) and the geometry never exists. forceload keeps the build
	# area loaded while the commands run.
	$fl = @($Cfg.forceload)
	if ($fl.Count -eq 4) {
		Invoke-VanillaConsole ("forceload add {0} {1} {2} {3}" -f $fl[0], $fl[1], $fl[2], $fl[3]) | Out-Null
		Start-Sleep -Milliseconds 1000
	}
	foreach ($cmd in $allCommands) { Invoke-VanillaConsole $cmd | Out-Null }
	Start-Sleep -Seconds 2

	# Alice must survive the deep pools: in run r2 (2026-11-16_1526) she drowned
	# in the 4-deep pool and the death screen froze every later teleport, so the
	# second half of the run recorded no geometry at all. water_breathing takes
	# drowning out of the picture and immediate respawn keeps a straggler death
	# from stalling the loop. Neither changes the pose/flag behaviour under test.
	Invoke-VanillaConsole ("effect give {0} minecraft:water_breathing infinite 0 true" -f $nickA) | Out-Null
	Invoke-VanillaConsole 'gamerule doImmediateRespawn true' | Out-Null
	Start-Sleep -Milliseconds 500

	foreach ($geo in $geometries) {
		$stand = $geo.stand
		if ($geo.enterFromTop) {
			# Entry from above: drop through the water surface before the stand
			# teleport, so the first samples are the fall-in, not a placement.
			# dropY stays above the surface: a 4-deep pool otherwise starts the
			# fall already submerged (run r2, 2026-11-16_1526).
			$drop = [string]::Format($inv, "tp {0} {1} {2} {3} {4} {5}", `
					$nickA, $stand.x, $geo.dropY, $stand.z, $stand.yaw, $stand.pitch)
			Invoke-StandRcon $drop | Out-Null
			Start-Sleep -Milliseconds 1200
		}
		$tp = [string]::Format($inv, "tp {0} {1} {2} {3} {4} {5}", `
				$nickA, $stand.x, $stand.y, $stand.z, $stand.yaw, $stand.pitch)
		# The teleport goes through RCON (the path S8 proved); the client is then
		# probed until it reports the stand point (a geometry with no floor makes
		# the player fall, and that must be visible as a failed check).
		Invoke-StandRcon $tp | Out-Null
		$tpOk = $false
		$probe = $null
		$tpDeadline = (Get-Date).AddSeconds(6)
		while ((Get-Date) -lt $tpDeadline) {
			Start-Sleep -Milliseconds 200
			$probe = (Invoke-TestCmd $portA (New-ObserveCmd "s10-$($geo.name)-tp-probe")).result
			if ($null -eq $probe -or $null -eq $probe.pos) { continue }
			$dxz = [Math]::Sqrt([Math]::Pow($probe.pos.x - $stand.x, 2) + [Math]::Pow($probe.pos.z - $stand.z, 2))
			$dy = [Math]::Abs($probe.pos.y - $stand.y)
			if ($dxz -le 1.0 -and $dy -le 2.0) { $tpOk = $true; break }
		}
		if (-not $tpOk) {
			$probeFact = if ($probe) { "({0},{1},{2})" -f $probe.pos.x, $probe.pos.y, $probe.pos.z } else { 'no observe' }
			Add-Check $Result ("S10 {0}: A reached the stand point" -f $geo.name) $false `
				("tp '$tp' did not apply; probe pos=$probeFact")
			continue
		}
		Start-Sleep -Milliseconds ([int]$Cfg.settleMs)
		$before = (Invoke-TestCmd $portA (New-ObserveCmd "s10-$($geo.name)-pre")).result
		$script:s7Facts.Add(("S10 {0} stand: pos=({1},{2},{3}) feet={4} head={5} eye={6} inWater={7} pose={8}" -f `
				$geo.name, $before.pos.x, $before.pos.y, $before.pos.z, `
				$before.feetBlock.id, $before.headBlock.id, $before.eyeBlock.id, `
				$before.isInWater, $before.pose))

		foreach ($action in @($Cfg.programs.$($geo.program))) {
			$id = "$($geo.name)-$($action.phase)"
			if ($action.key) {
				$null = Invoke-TestCmd $portA (New-KeyCmd $id ([string]$action.key) ([string]$action.action))
			}
			$remaining = [int]$action.ms
			while ($remaining -gt 0) {
				$slice = [Math]::Min($sampleEvery, $remaining)
				Start-Sleep -Milliseconds $slice
				$remaining -= $slice
				$elapsed = [int]$action.ms - $remaining
				$aResp = Invoke-TestCmd $portA (New-ObserveCmd "s10-a-$($script:s10SampleIndex)")
				$bResp = Invoke-TestCmd $portB (New-ObserveCmd "s10-b-$($script:s10SampleIndex)" $nickA)
				$script:s10SampleIndex++
				$script:s10Samples.Add([PSCustomObject]@{
						geometry  = [string]$geo.name
						phase     = [string]$action.phase
						elapsedMs = $elapsed
						a         = $(if ($aResp.ok) { $aResp.result } else { $null })
						b         = $(if ($bResp.ok) { $bResp.result } else { $null })
					}) | Out-Null
			}
		}

		# Per-geometry fact: the labels A and B reported across the program.
		$rows = @($script:s10Samples | Where-Object { $_.geometry -eq $geo.name })
		$aPoses = @($rows | ForEach-Object { $_.a.pose } | Sort-Object -Unique) -join ','
		$aInWater = @($rows | Where-Object { $_.a.isInWater -eq $true }).Count
		$bPoses = @($rows | ForEach-Object { $_.b.pose } | Sort-Object -Unique) -join ','
		$bSwim = @($rows | Where-Object { $_.b.isSwimming -eq $true }).Count
		Add-Check $Result ("S10 {0}: samples recorded" -f $geo.name) ($rows.Count -gt 0) `
			("samples={0}; A poses=[{1}] A isInWater samples={2}; B poses=[{3}] B isSwimming samples={4}" -f `
				$rows.Count, $aPoses, $aInWater, $bPoses, $bSwim)
		# Release every key so the next geometry starts from a clean input state.
		foreach ($key in @('up', 'down', 'left', 'right', 'jump', 'shift', 'sprint')) {
			$null = Invoke-TestCmd $portA (New-KeyCmd "$($geo.name)-release-$key" $key 'up')
		}
	}

	$artifact = [PSCustomObject]@{
		target        = $Target
		generated     = (Get-Date).ToString('s')
		baseY         = $baseY
		uuidA         = $launchA.uuid
		nickA         = $nickA
		nickB         = $nickB
		platform      = @($Cfg.platform)
		buildCommands = @($allCommands)
		geometries    = @($geometries | ForEach-Object { [PSCustomObject]@{ name = $_.name; kind = $_.kind; program = $_.program; enterFromTop = $_.enterFromTop; dropY = $_.dropY; stand = $_.stand; commands = @($_.commands) } })
		samples       = @($script:s10Samples)
	}
	$artifactPath = Join-Path $RunDir 's10-samples.json'
	$artifact | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $artifactPath -Encoding utf8
	$script:s7Facts.Add("S10 artifact: $artifactPath ($($script:s10Samples.Count) samples)")
	Write-Host ("  [info] S10 artifact written: $artifactPath")

	$null = Invoke-TestCmd $portA (New-SessionCmd 's10-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's10-offb' 'disconnect')
	Start-Sleep -Seconds 3
}

$scenarioDir = Join-Path $here 'scenarios'
$allResults = @()
try {
	Prepare-RunWorld
	Start-Stand
	foreach ($id in $Scenarios) {
		$file = Get-ChildItem $scenarioDir -Filter "$id-*.json" | Select-Object -First 1
		if (-not $file) { throw "no scenario file for '$id' in $scenarioDir" }
		$cfg = Get-Content $file.FullName -Raw | ConvertFrom-Json
		Write-Host ("=== {0}: {1} ===" -f $cfg.id, $cfg.title)
		$result = [PSCustomObject]@{
			id      = $cfg.id
			title   = $cfg.title
			failed  = $false
			checks  = [System.Collections.ArrayList]@()
			traces  = @()
		}
		$before = Get-Date
		# Clients must be dead before a scenario starts and after it ends:
		# a leftover process keeps the natives dir locked (Remove-Item in
		# New-ClientLaunch fails) and sits on the server-list screen.
		Stop-ClientByName $cfg.playerA
		Stop-ClientByName $cfg.playerB
		Start-Sleep -Seconds 2
		switch ($cfg.id) {
			's1' { Invoke-S1 $cfg $result }
			's1b' { Invoke-S1b $cfg $result }
			's2' { Invoke-S2 $cfg $result }
			's3' { Invoke-S3 $cfg $result }
			's4' { Invoke-S4 $cfg $result }
			's5' { Invoke-S5 $cfg $result }
			's6' { Invoke-S6 $cfg $result }
			's7' { Invoke-S7 $cfg $result }
			's8' { Invoke-S8 $cfg $result }
			's9' { Invoke-S9 $cfg $result }
			's10' { Invoke-S10 $cfg $result }
			default { throw "unknown scenario '$($cfg.id)'" }
		}
		Stop-ClientByName $cfg.playerA
		Stop-ClientByName $cfg.playerB
		Start-Sleep -Seconds 3
		$result.traces = @(Get-NewTraceFiles $traceDir $before | ForEach-Object { $_.Name })
		$allResults += $result
	}
} finally {
	if (-not $KeepStand) { Stop-Stand }
}

$summary = [PSCustomObject]@{
	runDir    = $RunDir
	target    = $Target
	generated = (Get-Date).ToString('s')
	runStartedAt = $script:runStartedAt.ToString('o')
	worldAudit = Join-Path $RunDir 'world-audit.json'
	scenarios = $allResults
}
$summary | ConvertTo-Json -Depth 6 | Set-Content $summaryPath -Encoding utf8
$script:s7Facts | Set-Content $summaryTextPath -Encoding utf8
Write-Host "summary: $summaryTextPath"
Write-Host "summary: $summaryPath"
$failedCount = @($allResults | Where-Object { $_.failed }).Count
Write-Host ("{0} scenario(s) run, {1} with failed checks" -f $allResults.Count, $failedCount)
if ($failedCount -gt 0) { exit 1 }
