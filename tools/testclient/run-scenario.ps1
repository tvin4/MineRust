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
	[switch]$KeepStand
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'scenario-lib.ps1')

# --- paths -----------------------------------------------------------------
$vanillaDir = 'E:\MineRust-data\vanilla'
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
		(Get-Content $repoServerToml -Raw) -replace 'listen\s*=\s*"[^"]*"', 'listen = "127.0.0.1:25566"' |
			Set-Content $oursConfig -Encoding utf8
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

	# Air phase: look straight up (crosshair can only hit sky) and tap
	# attack - a swing in air must be punch (46) without player_action (41).
	$null = Invoke-TestCmd $portA (New-LookCmd 's3-up' 0 -4000)
	for ($i = 0; $i -lt $airTaps; $i++) {
		$null = Invoke-TestCmd $portA (New-KeyCmd 's3-air' 'attack')
		Start-Sleep -Milliseconds $gapMs
	}

	# Dig phase: look straight down (at the block underfoot) and hold
	# attack - holding on a block keeps punching and adds player_action.
	$null = Invoke-TestCmd $portA (New-LookCmd 's3-down' 0 4000)
	Start-Sleep -Milliseconds 300
	$null = Invoke-TestCmd $portA (New-KeyCmd 's3-dig-on' 'attack' 'down')
	Start-Sleep -Milliseconds $digMs
	$null = Invoke-TestCmd $portA (New-KeyCmd 's3-dig-off' 'attack' 'up')
	Start-Sleep -Milliseconds 500

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
	$digPunches = @($punchSeqs | Where-Object { $_ -gt $firstAction })
	Add-Check $Result 'hold on block: player_action present and punch continues after it' `
		($actionSeqs.Count -ge 1 -and $digPunches.Count -ge 1) `
		("action seqs: [{0}]; punch seqs after first action: [{1}]" -f ($actionSeqs -join ','), ($digPunches -join ','))

	# A->B visibility: the swing broadcast (clientbound swing_animation)
	# reaches B; A's own trace never carries it (broadcast excludes self).
	$bSwings = 0
	if ($bTrace) {
		$bSwings = @(Read-Trace $bTrace.FullName |
			Where-Object { $_.dir -eq 'clientbound' -and $_.name -match 'swing_animation|animate' }).Count
	}
	Add-Check $Result 'B receives swing broadcast while A swings' ($bSwings -ge 1) `
		("clientbound swing_animation/animate rows in B's trace ({0}): {1}" -f $bTrace.Name, $bSwings)
}

# --- S5: sneak and sprint flags seen by B (stage 10, encoder B reference) ---
# A holds shift / sprint, then releases. Facts come from the traces: the
# serverbound player_input flags in A's trace vs the clientbound
# set_entity_data rows for A's entity id in B's trace (index, type, value).
# S5 asserts the trace-confirmed state payloads and the sprint attribute order.
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
		Invoke-VanillaConsole $tpCommand | Out-Null
		$script:s7Facts.Add("$($pose.name): teleport command sent via vanilla stdin: $tpCommand")
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
		$null = Invoke-TestCmd $portB (New-ObserveCmd ('s7-' + $pose.name + '-observe') $nickA)
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
	foreach ($pose in $Cfg.poses) {
		if ($pose.alsoKey) {
			$null = Invoke-TestCmd $portA (New-KeyCmd ('s5-' + $pose.name + '-also-on') $pose.alsoKey 'down')
		}
		$null = Invoke-TestCmd $portA (New-KeyCmd ('s5-' + $pose.name + '-on') $pose.key 'down')
		Start-Sleep -Milliseconds ([int]$pose.holdMs)
		$null = Invoke-TestCmd $portA (New-KeyCmd ('s5-' + $pose.name + '-off') $pose.key 'up')
		if ($pose.alsoKey) {
			$null = Invoke-TestCmd $portA (New-KeyCmd ('s5-' + $pose.name + '-also-off') $pose.alsoKey 'up')
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

	$attributes = @($bRows | Where-Object {
		if ($_.dir -ne 'clientbound' -or $_.name -ne 'minecraft:update_attributes') { return $false }
		$p = ConvertFrom-TraceB64 $_.b64
		$p.Length -ge 3 -and $p[0] -eq $entityId -and $p[1] -eq 1 -and $p[2] -eq 0x1a
	} | ForEach-Object { $p = ConvertFrom-TraceB64 $_.b64; [PSCustomObject]@{seq=[int]$_.seq; hex=[BitConverter]::ToString($p).Replace('-', ' ').ToLowerInvariant(); sprint=([Text.Encoding]::ASCII.GetString($p).Contains('minecraft:sprinting'))} })
	$onAttribute = $null
	$offAttribute = $null
	if ($stateMatch) {
		$onAttribute = $attributes | Where-Object { $_.seq -gt $found[2].seq -and $_.sprint } | Select-Object -First 1
		$offAttribute = $attributes | Where-Object { $_.seq -gt $found[3].seq -and -not $_.sprint } | Select-Object -First 1
	}
	$attributeSuffix = if ($entityId -lt 0x80) { '{0:x2}' -f $entityId } else { $null }
	$onHex = $attributeSuffix + ' 01 1a 3f b9 99 99 a0 00 00 00 01 13 6d 69 6e 65 63 72 61 66 74 3a 73 70 72 69 6e 74 69 6e 67 3f d3 33 33 40 00 00 00 02'
	$offHex = $attributeSuffix + ' 01 1a 3f b9 99 99 a0 00 00 00 00'
	$onPass = $null -ne $onAttribute -and $onAttribute.hex -eq $onHex -and $onAttribute.seq -gt $found[2].seq
	$offPass = $null -ne $offAttribute -and $offAttribute.hex -eq $offHex -and $offAttribute.seq -gt $found[3].seq
	$onDetail = 'missing'
	if ($onAttribute) { $onDetail = "seq=$($onAttribute.seq) hex=$($onAttribute.hex)" }
	$offDetail = 'missing'
	if ($offAttribute) { $offDetail = "seq=$($offAttribute.seq) hex=$($offAttribute.hex)" }
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

# --- main ------------------------------------------------------------------
$scenarioDir = Join-Path $here 'scenarios'
$allResults = @()
try {
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
	scenarios = $allResults
}
$summary | ConvertTo-Json -Depth 6 | Set-Content $summaryPath -Encoding utf8
$script:s7Facts | Set-Content $summaryTextPath -Encoding utf8
Write-Host "summary: $summaryTextPath"
Write-Host "summary: $summaryPath"
$failedCount = @($allResults | Where-Object { $_.failed }).Count
Write-Host ("{0} scenario(s) run, {1} with failed checks" -f $allResults.Count, $failedCount)
if ($failedCount -gt 0) { exit 1 }
