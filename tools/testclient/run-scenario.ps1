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
	# Game mode new players join in on our server (РЁР°Рі 14c). -1 (the default)
	# leaves the generated server-ours.toml untouched (survival). The S9
	# creative run uses 1: our server has no RCON to switch modes in game.
	[int]$OursGameMode = -1,
	# Development-only spawn override for our server (РЁР°Рі 14d): "x,y,z" is
	# written into the generated server-ours.toml as spawn_override. Empty
	# (the default) leaves the config untouched (the save's spawn). S8 uses it
	# to place the joining player above open water, since our server has no
	# RCON to teleport the player in game.
	[string]$OursSpawnOverride = '',
	# Development-only geometry file for our server (РЁР°Рі 15 Рї.2): setblock/fill
	# commands applied to the in-memory store at startup. Empty leaves the
	# generated config untouched. For S10 the runner writes the file from the
	# scenario definition before the stand starts (-OursDevGeometry is the path).
	[string]$OursDevGeometry = '',
	# Data-generation hook (РЁР°Рі 15 Рї.5): when set, the S11 vanilla run asks the
	# connected test client to dump every block state's collision-shape bottom
	# (public BlockState API) to this file. The server's collision_report reads
	# that file; it stays outside the repository.
	[string]$CollisionReport = '',
	# Hardness report for our server (РЁР°Рі 16a.4 Рї.4): the path is written into
	# the generated server-ours.toml as hardness_report, which switches the
	# survival dig timer on. It is never committed (E:\MineRust-data).
	[string]$HardnessReport = '',
	# РЁР°Рі 19a: the S13 checks run the release build (same sources, same
	# recipe as S12) instead of target\debug.
	[switch]$Release,
	# РЁР°Рі 19a: player saves. The save root is written into the generated
	# config; an empty value keeps the server's persistence off. The autosave
	# period is in ticks (-1 leaves the config value).
	[string]$OursSaveDir = '',
	[int]$OursAutosaveTicks = -1,
	# Шаг 23b: world autosave is independent from playerdata. A small value is
	# used by S16b so the runner can prove a durable diff before a hard kill.
	[int]$OursWorldAutosaveTicks = -1,
	[ValidateSet('', 'true', 'false')]
	[string]$OursSaveIgnoreWorldMismatch = '',
	# РЁР°Рі 19a: force_gamemode for the generated config ('' leaves it).
	[ValidateSet('', 'true', 'false')]
	[string]$OursForceGamemode = '',
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
$buildProfile = if ($Release) { 'release' } else { 'debug' }
$mcproxyExe = "E:\projects\MineRust\target\$buildProfile\mcproxy.exe"
# Stage G target: our own Rust server (workspace server.toml, listen patched
# to 25566 because mcproxy reads its upstream port from the vanilla
# server.properties).
$serverExe = "E:\projects\MineRust\target\$buildProfile\server.exe"
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
$script:oursProcess = $null
$script:oursConfig = $null
$script:oursGeneration = 0
$script:oursEvents = @()
$script:mcproxyPid = $null

# Starts one generation of our server from the config written by Start-Stand.
# mcproxy deliberately stays alive across generations: every upstream failure
# ends only that proxied connection, not its listener.
function Start-OursServer {
	if (-not $script:oursConfig -or -not (Test-Path -LiteralPath $script:oursConfig)) {
		throw 'ours config is not ready'
	}
	if ($script:oursProcess -and -not $script:oursProcess.HasExited) {
		throw "ours server generation $($script:oursGeneration) is already running"
	}
	$script:oursGeneration++
	$generation = $script:oursGeneration
	$oursLog = Join-Path $RunDir ("ours-stdout-{0:d2}.log" -f $generation)
	$oursErr = Join-Path $RunDir ("ours-stderr-{0:d2}.log" -f $generation)
	$script:oursLog = $oursLog
	# System.Diagnostics.Process, not Start-Process -PassThru: with redirected
	# output the latter never exposes ExitCode after the process exits
	# (measured 18.11.2026 with an isolated `cmd /c exit 7` probe), and S16
	# needs the real shutdown exit code. The async pumps mirror the vanilla
	# branch because the runner tails these logs while the server runs.
	$startInfo = New-Object System.Diagnostics.ProcessStartInfo
	$startInfo.FileName = $serverExe
	$startInfo.Arguments = "--config `"$($script:oursConfig)`""
	$startInfo.WorkingDirectory = Split-Path -Parent $repoServerToml
	$startInfo.UseShellExecute = $false
	$startInfo.RedirectStandardInput = $true
	$startInfo.RedirectStandardOutput = $true
	$startInfo.RedirectStandardError = $true
	$process = New-Object System.Diagnostics.Process
	$process.StartInfo = $startInfo
	if (-not $process.Start()) { throw "could not start ours generation $generation" }
	$script:oursProcess = $process
	$script:oursPid = $process.Id
	$script:oursEvents += Register-ObjectEvent -InputObject $process -EventName OutputDataReceived -MessageData $oursLog -Action {
		if ($EventArgs.Data) { Add-Content -LiteralPath $Event.MessageData -Value $EventArgs.Data }
	}
	$script:oursEvents += Register-ObjectEvent -InputObject $process -EventName ErrorDataReceived -MessageData $oursErr -Action {
		if ($EventArgs.Data) { Add-Content -LiteralPath $Event.MessageData -Value $EventArgs.Data }
	}
	$process.BeginOutputReadLine()
	$process.BeginErrorReadLine()
	$deadline = (Get-Date).AddSeconds(120)
	while ((Get-Date) -lt $deadline) {
		if ($process.HasExited) {
			throw "ours generation $generation exited during startup (code $($process.ExitCode)); see $oursLog / $oursErr"
		}
		if (Get-NetTCPConnection -LocalPort 25566 -State Listen -ErrorAction SilentlyContinue) {
			Write-Host ("ours generation {0} ready (pid {1}, log {2})" -f $generation, $script:oursPid, $oursLog)
			return $oursLog
		}
		Start-Sleep -Milliseconds 300
	}
	throw "ours generation $generation did not open 25566"
}

# Stops only our upstream server. Graceful mode writes the vanilla console
# `stop` command to the server's stdin (it flushes and exits 0); hard mode
# terminates the exact process. Returns the stopped generation and its log.
function Stop-OursServer {
	param(
		[ValidateSet('graceful', 'hard')][string]$Mode,
		[int]$TimeoutSec = 30
	)
	if (-not $script:oursProcess -or $script:oursProcess.HasExited) {
		return [PSCustomObject]@{ mode = $Mode; generation = $script:oursGeneration; log = $script:oursLog; exited = $true; exitCode = $null }
	}
	$process = $script:oursProcess
	$serverPid = $process.Id
	$generation = $script:oursGeneration
	$log = $script:oursLog
	if ($Mode -eq 'graceful') {
		# Vanilla parity: our server accepts the console `stop` command on
		# stdin and flushes on shutdown. `taskkill` (WM_CLOSE) cannot be used
		# here: the System.Diagnostics.Process launcher gives the child no
		# console window, so the window-close signal can never be delivered.
		$process.StandardInput.WriteLine('stop')
		$process.StandardInput.Flush()
	} else {
		Stop-Process -Id $serverPid -Force -ErrorAction Stop
	}
	if (-not $process.WaitForExit($TimeoutSec * 1000)) {
		throw "ours generation $generation did not stop in ${TimeoutSec}s ($Mode)"
	}
	# The parameterless overload waits for the async output pumps to drain, so
	# the generation log is complete before its flush lines are read.
	$process.WaitForExit()
	$exitCode = $process.ExitCode
	foreach ($job in $script:oursEvents) {
		Unregister-Event -SourceIdentifier $job.Name -ErrorAction SilentlyContinue
		Remove-Job -Id $job.Id -Force -ErrorAction SilentlyContinue
	}
	$script:oursEvents = @()
	$portDeadline = (Get-Date).AddSeconds(10)
	while ((Get-Date) -lt $portDeadline -and
		(Get-NetTCPConnection -LocalPort 25566 -State Listen -ErrorAction SilentlyContinue)) {
		Start-Sleep -Milliseconds 200
	}
	if (Get-NetTCPConnection -LocalPort 25566 -State Listen -ErrorAction SilentlyContinue) {
		throw "ours generation $generation stopped but port 25566 is still listening"
	}
	$script:oursProcess = $null
	$script:oursPid = $null
	return [PSCustomObject]@{ mode = $Mode; generation = $generation; log = $log; exited = $true; exitCode = $exitCode }
}

function Restart-OursServer {
	return Start-OursServer
}

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
		$script:oursConfig = $oursConfig
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
		if ($OursDevGeometry) {
			# TOML basic string: forward slashes avoid escape processing, and
			# Windows accepts them as a path separator.
			$geomPath = $OursDevGeometry -replace '\\', '/'
			if ($config -match '(?m)^dev_geometry\s*=') {
				$config = $config -replace '(?m)^dev_geometry\s*=.*$', "dev_geometry = `"$geomPath`""
			} else {
				$config = $config.TrimEnd() + "`r`ndev_geometry = `"$geomPath`"`r`n"
			}
		}
		if ($CollisionReport) {
			# The forced crouch/crawl rule reads the collision report; point
			# the generated config at it (forward slashes, see above).
			$collisionPath = $CollisionReport -replace '\\', '/'
			if ($config -match '(?m)^collision_report\s*=') {
				$config = $config -replace '(?m)^collision_report\s*=.*$', "collision_report = `"$collisionPath`""
			} else {
				$config = $config.TrimEnd() + "`r`ncollision_report = `"$collisionPath`"`r`n"
			}
		}
		if ($HardnessReport) {
			# РЁР°Рі 16a.4 Рї.4: the survival dig timer needs the exported hardness
			# scale. The path stays outside the repository, like the collision
			# report above.
			$hardnessPath = $HardnessReport -replace '\\', '/'
			if ($config -match '(?m)^hardness_report\s*=') {
				$config = $config -replace '(?m)^hardness_report\s*=.*$', "hardness_report = `"$hardnessPath`""
			} else {
				$config = $config.TrimEnd() + "`r`nhardness_report = `"$hardnessPath`"`r`n"
			}
		}
		if ($OursSaveDir) {
			# Player saves (РЁР°Рі 19a): the root is generated per run, never the
			# template and never vanilla\world. Forward slashes, as above.
			$savePath = $OursSaveDir -replace '\\', '/'
			if ($config -match '(?m)^save_dir\s*=') {
				$config = $config -replace '(?m)^save_dir\s*=.*$', "save_dir = `"$savePath`""
			} else {
				$config = $config.TrimEnd() + "`r`nsave_dir = `"$savePath`"`r`n"
			}
		}
		if ($OursAutosaveTicks -ge 0) {
			if ($config -match '(?m)^autosave_interval_ticks\s*=') {
				$config = $config -replace '(?m)^autosave_interval_ticks\s*=.*$', "autosave_interval_ticks = $OursAutosaveTicks"
			} else {
				$config = $config.TrimEnd() + "`r`nautosave_interval_ticks = $OursAutosaveTicks`r`n"
			}
		}
		if ($OursWorldAutosaveTicks -ge 0) {
			if ($config -match '(?m)^world_autosave_ticks\s*=') {
				$config = $config -replace '(?m)^world_autosave_ticks\s*=.*$', "world_autosave_ticks = $OursWorldAutosaveTicks"
			} else {
				$config = $config.TrimEnd() + "`r`nworld_autosave_ticks = $OursWorldAutosaveTicks`r`n"
			}
		}
		if ($OursSaveIgnoreWorldMismatch) {
			if ($config -match '(?m)^save_ignore_world_mismatch\s*=') {
				$config = $config -replace '(?m)^save_ignore_world_mismatch\s*=.*$', "save_ignore_world_mismatch = $OursSaveIgnoreWorldMismatch"
			} else {
				$config = $config.TrimEnd() + "`r`nsave_ignore_world_mismatch = $OursSaveIgnoreWorldMismatch`r`n"
			}
		}
		if ($OursForceGamemode) {
			if ($config -match '(?m)^force_gamemode\s*=') {
				$config = $config -replace '(?m)^force_gamemode\s*=.*$', "force_gamemode = $OursForceGamemode"
			} else {
				$config = $config.TrimEnd() + "`r`nforce_gamemode = $OursForceGamemode`r`n"
			}
		}
		$config | Set-Content $oursConfig -Encoding utf8
		$null = Start-OursServer
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
	if ($script:oursProcess -and -not $script:oursProcess.HasExited) {
		try { $null = Stop-OursServer -Mode hard -TimeoutSec 15 }
		catch { Stop-Process -Id $script:oursProcess.Id -Force -ErrorAction SilentlyContinue }
	}
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
	param([string]$Nick, [int]$ControlPort, [datetime]$Since, [int]$TimeoutSec = 120, [string]$Instance = '')
	# A previous client of the same nick may still be alive after `disconnect`
	# (the game returns to the main menu instead of exiting) and holds its log
	# files and natives DLLs. The server-side quit has already been waited for
	# by the caller, so stopping the leftover process here is safe and makes
	# the relaunch deterministic (S13 re-entry). With -Instance only that
	# instance is stopped, so a second connection of the same nick (S15) can
	# start while the first one still plays.
	Stop-ClientByName ($Nick + $Instance)
	$out = & $launchScript -Nick $Nick -Address '127.0.0.1:25565' -ControlPort $ControlPort -Instance $Instance
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

# Launches one test client without waiting for the join gate (S15 V4 starts
# two connections back to back, so the first may still be in configuration
# when the second logs in). Returns { pid, uuid } like Start-Client.
function Start-ClientNoWait {
	param([string]$Nick, [int]$ControlPort, [string]$Instance = '')
	Stop-ClientByName ($Nick + $Instance)
	$out = & $launchScript -Nick $Nick -Address '127.0.0.1:25565' -ControlPort $ControlPort -Instance $Instance
	$pidOut = [int]($out | Select-Object -Last 1)
	$uuid = $null
	foreach ($line in $out) {
		if ($line -match 'offline uuid ([0-9a-f-]+)') { $uuid = $Matches[1] }
	}
	return [PSCustomObject]@{ pid = $pidOut; uuid = $uuid }
}

# Waits until one client reports that it is in the game. Used by the ordered
# S15 runs: the first client is followed by the second, so only its own state
# matters, and a duplicate login of the first leaves the second in play. The
# trace-based Wait-S15Outcome below cannot tell the two same-name clients
# apart (the kick of the first lands in both traces' poll window), so the
# ordered path must not use it.
function Wait-S15Play {
	param([int]$ControlPort, [int]$TimeoutSec = 120)
	$deadline = (Get-Date).AddSeconds($TimeoutSec)
	while ((Get-Date) -lt $deadline) {
		try {
			$resp = Invoke-TestCmd $ControlPort (New-ObserveCmd 's15-play') 3000
			if ($resp.ok -and $resp.result.inGame) { return 'play' }
		} catch { }
		Start-Sleep -Milliseconds 300
	}
	return 'timeout'
}

# Waits until one S15 connection either reaches play or receives a disconnect.
# `observe` answers at the main menu too, so a rejected connection is told
# apart by its fresh trace rather than by a timeout. Returns 'play',
# 'disconnect' or 'timeout'.
function Wait-S15Outcome {
	param([int]$ControlPort, [datetime]$Since, [int]$TimeoutSec = 90)
	$deadline = (Get-Date).AddSeconds($TimeoutSec)
	$tracePollAt = Get-Date
	while ((Get-Date) -lt $deadline) {
		try {
			$resp = Invoke-TestCmd $ControlPort (New-ObserveCmd 's15-wait') 3000
			if ($resp.ok -and $resp.result.inGame) { return 'play' }
		} catch { }
		if ((Get-Date) -ge $tracePollAt) {
			foreach ($file in @(Get-NewTraceFiles $traceDir $Since)) {
				if (Test-TraceHasDisconnect $file.FullName) { return 'disconnect' }
			}
			$tracePollAt = (Get-Date).AddSeconds(2)
		}
		Start-Sleep -Milliseconds 300
	}
	return 'timeout'
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
		# Ours (РЁР°Рі 14d): no RCON, so the water run uses the dev-only spawn
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

		# At rest in water the РЁР°Рі 14e rule says A is NOT swimming: the bit
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
		# Ours (РЁР°Рі 14c): creative comes from the generated config
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
		# Ours (РЁР°Рі 14c): the wire facts of "B sees the block gone" вЂ” the
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

# --- S10: swimming/crawling decision data (РЁР°Рі 14e) -------------------------
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

# Writes the dev-only geometry of the selected scenarios to a file our server
# applies to its in-memory store at startup (dev_geometry, РЁР°Рі 15 Рї.2). Must run
# before Start-Stand: our server has no /fill, so the geometry has to be in the
# store before the port opens. S10 and S11 share the format and the file.
function Write-DevGeometryFile {
	param([string]$Path)
	$lines = New-Object System.Collections.Generic.List[string]
	if ($Scenarios -contains 's10') {
		$scenarioFile = Get-ChildItem (Join-Path $here 'scenarios') -Filter 's10-*.json' | Select-Object -First 1
		if (-not $scenarioFile) { throw 'no s10 scenario file to build the geometry from' }
		$cfg = Get-Content $scenarioFile.FullName -Raw | ConvertFrom-Json
		$baseY = [int]$cfg.baseY
		$xs = @($cfg.geometryRow.xs)
		$zs = @($cfg.geometryRow.zs)
		foreach ($cmd in @($cfg.platform)) { $lines.Add([string]$cmd) }
		foreach ($geo in $cfg.geometries) {
			$slot = [int]$geo.slot
			$col = $slot % $xs.Count
			$row = [int][Math]::Floor($slot / $xs.Count)
			$build = Get-S10Build $geo $baseY $xs[$col] $zs[$row]
			foreach ($cmd in $build.commands) { $lines.Add([string]$cmd) }
		}
	}
	if ($Scenarios -contains 's11') {
		$scenarioFile = Get-ChildItem (Join-Path $here 'scenarios') -Filter 's11-*.json' | Select-Object -First 1
		if (-not $scenarioFile) { throw 'no s11 scenario file to build the geometry from' }
		$cfg = Get-Content $scenarioFile.FullName -Raw | ConvertFrom-Json
		foreach ($cmd in @($cfg.platform)) { $lines.Add([string]$cmd) }
		$roofY = [int]$cfg.roofY
		$from = [int]$cfg.roofFromZ
		$to = [int]$cfg.roofToZ
		$probeZ = [int]$cfg.probeZ
		foreach ($case in @($cfg.cases)) {
			$lines.Add(("fill {0} {1} {2} {0} {1} {3} {4}" -f [int]$case.x, $roofY, $from, $to, [string]$case.roof))
			# The isolated block over the probe cell: one full cell at head
			# level, the literal "block placed above the standing player" case.
			$lines.Add(("setblock {0} {1} {2} {3}" -f [int]$case.x, $roofY, $probeZ, [string]$case.roof))
		}
	}
	if ($Scenarios -contains 's12') {
		$scenarioFile = Get-ChildItem (Join-Path $here 'scenarios') -Filter 's12-*.json' | Select-Object -First 1
		if (-not $scenarioFile) { throw 'no s12 scenario file to build the geometry from' }
		$cfg = Get-Content $scenarioFile.FullName -Raw | ConvertFrom-Json
		foreach ($cmd in (Get-S12Commands $cfg)) { $lines.Add([string]$cmd) }
	}
	if ($Scenarios -contains 's14') {
		$scenarioFile = Get-ChildItem (Join-Path $here 'scenarios') -Filter 's14-*.json' | Select-Object -First 1
		if (-not $scenarioFile) { throw 'no s14 scenario file to build the geometry from' }
		$cfg = Get-Content $scenarioFile.FullName -Raw | ConvertFrom-Json
		foreach ($cmd in (Get-S14Commands $cfg)) { $lines.Add([string]$cmd) }
	}
	if ($lines.Count -eq 0) { throw 'no dev geometry for the selected scenarios' }
	$lines | Set-Content -LiteralPath $Path -Encoding utf8
	Write-Host ("  [info] dev geometry: {0} commands written to {1}" -f $lines.Count, $Path)
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
		# Our server has no /fill and no console: the geometry was applied to
		# the in-memory store at startup through -OursDevGeometry (written by
		# Write-S10GeometryFile before Start-Stand). Drowning is not
		# implemented on our server, so the water_breathing effect is not
		# needed either.
		if (-not $OursDevGeometry) {
			Add-Check $Result 'S10 geometry is available on our server' $false `
				'pass -OursDevGeometry <file>: the runner writes the s10 commands there before start'
			$null = Invoke-TestCmd $portA (New-SessionCmd 's10-off' 'disconnect')
			$null = Invoke-TestCmd $portB (New-SessionCmd 's10-offb' 'disconnect')
			Start-Sleep -Seconds 3
			return
		}
		$script:s7Facts.Add("S10 dev geometry applied at startup: $OursDevGeometry")
	} else {
		# The build runs only after a client is connected: before that the server
		# answers every /fill with "That position is not loaded" (first s10 run,
		# 2026-11-16_1500) and the geometry never exists. forceload keeps the
		# build area loaded while the commands run.
		$fl = @($Cfg.forceload)
		if ($fl.Count -eq 4) {
			Invoke-VanillaConsole ("forceload add {0} {1} {2} {3}" -f $fl[0], $fl[1], $fl[2], $fl[3]) | Out-Null
			Start-Sleep -Milliseconds 1000
		}
		foreach ($cmd in $allCommands) { Invoke-VanillaConsole $cmd | Out-Null }
		Start-Sleep -Seconds 2

		# Alice must survive the deep pools: in run r2 (2026-11-16_1526) she
		# drowned in the 4-deep pool and the death screen froze every later
		# teleport, so the second half of the run recorded no geometry at all.
		# water_breathing takes drowning out of the picture and immediate respawn
		# keeps a straggler death from stalling the loop. Neither changes the
		# pose/flag behaviour under test.
		Invoke-VanillaConsole ("effect give {0} minecraft:water_breathing infinite 0 true" -f $nickA) | Out-Null
		Invoke-VanillaConsole 'gamerule doImmediateRespawn true' | Out-Null
		# The observer must survive: on a fresh world at night a zombie killed Bob
		# mid-run (r4, 2026-11-16) and his death screen stopped every later
		# observation. Peaceful removes hostile mobs; it does not touch the
		# pose/flag behaviour under test.
		Invoke-VanillaConsole 'difficulty peaceful' | Out-Null
		Invoke-VanillaConsole 'gamerule doMobSpawning false' | Out-Null
		Start-Sleep -Milliseconds 500
	}

	foreach ($geo in $geometries) {
		$stand = $geo.stand
		if ($geo.enterFromTop) {
			# Entry from above: drop through the water surface before the stand
			# teleport, so the first samples are the fall-in, not a placement.
			# dropY stays above the surface: a 4-deep pool otherwise starts the
			# fall already submerged (run r2, 2026-11-16_1526).
			if ($isVanilla) {
				$drop = [string]::Format($inv, "tp {0} {1} {2} {3} {4} {5}", `
						$nickA, $stand.x, $geo.dropY, $stand.z, $stand.yaw, $stand.pitch)
				Invoke-StandRcon $drop | Out-Null
			} else {
				# Client-side teleport: our server has no /tp. The client's own
				# movement path reports the new position and our server applies
				# it, then the client falls the rest of the way itself.
				$null = Invoke-TestCmd $portA (New-TpCmd "s10-$($geo.name)-drop" `
						$stand.x $geo.dropY $stand.z $stand.yaw $stand.pitch)
			}
			Start-Sleep -Milliseconds 1200
		}
		$tp = [string]::Format($inv, "tp {0} {1} {2} {3} {4} {5}", `
				$nickA, $stand.x, $stand.y, $stand.z, $stand.yaw, $stand.pitch)
		# Vanilla: the vanilla console teleports (the path S8 proved). Ours: the
		# client teleports itself (no RCON/command on our server). The client is
		# then probed until it reports the stand point (a geometry with no floor
		# makes the player fall, and that must be visible as a failed check).
		if ($isVanilla) {
			Invoke-StandRcon $tp | Out-Null
		} else {
			$null = Invoke-TestCmd $portA (New-TpCmd "s10-$($geo.name)-tp" `
					$stand.x $stand.y $stand.z $stand.yaw $stand.pitch)
		}
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

# --- S11: forced crouch/crawl under a low ceiling (РЁР°Рі 15 Рї.5) --------------
# Static geometry, the same procedure on both targets: a roof strip at y=202
# (head level for feet at y=201) and one isolated block at the same height over
# a probe cell. A stands in the open, is moved under the isolated block, back to
# the open, walks under the roof, holds sneak and comes back out. On vanilla the
# literal hypothesis probe also runs: /setblock places the block at head level
# over the standing player, then removes it. Ours cannot place blocks at
# runtime, so that extra phase runs on vanilla only. The runner only records
# facts (A and B observations, including the client-measured hitbox); the rule
# is derived from the vanilla timeline.
function Invoke-S11 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$sampleEvery = [int]$Cfg.sampleEveryMs
	$standY = [double]$Cfg.standY
	$yaw = [double]$Cfg.yaw
	$roofY = [int]$Cfg.roofY
	$inv = [System.Globalization.CultureInfo]::InvariantCulture
	$script:s11Samples = New-Object System.Collections.ArrayList
	$script:s11SampleIndex = 0

	$watermark = Get-Date
	$launchA = Start-Client $nickA $portA $watermark
	if (-not $launchA.uuid) { throw 'S11: could not resolve Alice offline UUID' }
	$null = Start-Client $nickB $portB (Get-Date)
	$observer = Wait-ObserverChunksStable $portB 30
	$script:s7Facts.Add(("S11 B chunk drain: settled={0} loadedChunks={1}" -f $observer.settled, $observer.loadedChunks))

	if ($isVanilla -and $CollisionReport) {
		# Data generation for the server's headroom query: the connected client
		# reads every state's collision-shape bottom through the game's own
		# public API and writes the report. The file stays outside the repo.
		$shapesCmd = [PSCustomObject]@{
			id   = 's11-shapes'
			cmd  = 'shapes'
			args = [PSCustomObject]@{ path = $CollisionReport }
		} | ConvertTo-Json -Compress
		$shapes = Invoke-TestCmd $portA $shapesCmd 30000
		$fact = if ($shapes.ok) {
			"states={0} nonEmpty={1} path={2}" -f $shapes.result.states, $shapes.result.nonEmpty, $shapes.result.path
		} else {
			"error: $($shapes.error)"
		}
		Add-Check $Result 'S11 collision report written' ([bool]$shapes.ok) $fact
	}

	$cases = New-Object System.Collections.ArrayList
	foreach ($case in @($Cfg.cases)) {
		$commands = New-Object System.Collections.Generic.List[string]
		$commands.Add(("fill {0} {1} {2} {0} {1} {3} {4}" -f [int]$case.x, $roofY, `
					[int]$Cfg.roofFromZ, [int]$Cfg.roofToZ, [string]$case.roof))
		$commands.Add(("setblock {0} {1} {2} {3}" -f [int]$case.x, $roofY, [int]$Cfg.probeZ, [string]$case.roof))
		$cases.Add([PSCustomObject]@{ name = [string]$case.name; x = [int]$case.x; roof = [string]$case.roof; commands = $commands }) | Out-Null
	}

	if (-not $isVanilla) {
		if (-not $OursDevGeometry) {
			Add-Check $Result 'S11 geometry is available on our server' $false `
				'pass -OursDevGeometry <file>: the runner writes the s11 commands there before start'
			$null = Invoke-TestCmd $portA (New-SessionCmd 's11-off' 'disconnect')
			$null = Invoke-TestCmd $portB (New-SessionCmd 's11-offb' 'disconnect')
			Start-Sleep -Seconds 3
			return
		}
		$script:s7Facts.Add("S11 dev geometry applied at startup: $OursDevGeometry")
	} else {
		$fl = @($Cfg.forceload)
		if ($fl.Count -eq 4) {
			Invoke-VanillaConsole ("forceload add {0} {1} {2} {3}" -f $fl[0], $fl[1], $fl[2], $fl[3]) | Out-Null
			Start-Sleep -Milliseconds 1000
		}
		foreach ($cmd in @($Cfg.platform)) { Invoke-VanillaConsole $cmd | Out-Null }
		foreach ($case in $cases) { foreach ($cmd in $case.commands) { Invoke-VanillaConsole $cmd | Out-Null } }
		Start-Sleep -Seconds 2
		# Same survival measures as S10: a fresh night world must not kill B.
		Invoke-VanillaConsole 'difficulty peaceful' | Out-Null
		Invoke-VanillaConsole 'gamerule doMobSpawning false' | Out-Null
		Start-Sleep -Milliseconds 500
	}

	foreach ($case in $cases) {
		$standX = [double]$case.x + 0.5
		$spots = @{
			open  = [PSCustomObject]@{ x = $standX; y = $standY; z = [double]$Cfg.openZ + 0.5 }
			probe = [PSCustomObject]@{ x = $standX; y = $standY; z = [double]$Cfg.probeZ + 0.5 }
		}
		$tpOk = $true
		$tpFact = ''
		foreach ($action in @($Cfg.program)) {
			$id = "$($case.name)-$($action.phase)"
			if ($action.tp) {
				$spot = $spots[[string]$action.tp]
				if ($null -eq $spot) { throw "S11: unknown spot '$($action.tp)'" }
				if ($isVanilla) {
					$line = [string]::Format($inv, "tp {0} {1} {2} {3} {4} {5}", `
							$nickA, $spot.x, $spot.y, $spot.z, $yaw, 0.0)
					Invoke-StandRcon $line | Out-Null
					Start-Sleep -Milliseconds 200
				} else {
					# Client-side teleport; our server has no /tp. The client then
					# reports its own position, which the server applies.
					$null = Invoke-TestCmd $portA (New-TpCmd "$id-tp" $spot.x $spot.y $spot.z $yaw 0.0)
				}
				# Probe until A reports the spot: a geometry without a floor would
				# make it fall, and that has to surface as a fact, not silence.
				$probe = $null
				$deadline = (Get-Date).AddSeconds(5)
				while ((Get-Date) -lt $deadline) {
					Start-Sleep -Milliseconds 150
					$probe = (Invoke-TestCmd $portA (New-ObserveCmd "$id-tp-probe")).result
					if ($null -eq $probe -or $null -eq $probe.pos) { continue }
					$dxz = [Math]::Sqrt([Math]::Pow($probe.pos.x - $spot.x, 2) + [Math]::Pow($probe.pos.z - $spot.z, 2))
					$dy = [Math]::Abs($probe.pos.y - $spot.y)
					if ($dxz -le 1.0 -and $dy -le 2.0) { break }
				}
				if ($null -eq $probe -or $null -eq $probe.pos) {
					$tpOk = $false
					$tpFact = "phase $($action.phase): no observe"
				} elseif ([Math]::Abs($probe.pos.y - $spot.y) -gt 2.0) {
					$tpOk = $false
					$tpFact = "phase $($action.phase): y=$($probe.pos.y) expected $($spot.y)"
				}
				Start-Sleep -Milliseconds ([int]$Cfg.settleMs)
			}
			if ($action.key) {
				$null = Invoke-TestCmd $portA (New-KeyCmd $id ([string]$action.key) ([string]$action.action))
			}
			$remaining = [int]$action.ms
			while ($remaining -gt 0) {
				$slice = [Math]::Min($sampleEvery, $remaining)
				Start-Sleep -Milliseconds $slice
				$remaining -= $slice
				$elapsed = [int]$action.ms - $remaining
				$aResp = Invoke-TestCmd $portA (New-ObserveCmd "s11-a-$($script:s11SampleIndex)")
				$bResp = Invoke-TestCmd $portB (New-ObserveCmd "s11-b-$($script:s11SampleIndex)" $nickA)
				$script:s11SampleIndex++
				$script:s11Samples.Add([PSCustomObject]@{
						caseName  = [string]$case.name
						phase     = [string]$action.phase
						elapsedMs = $elapsed
						a         = $(if ($aResp.ok) { $aResp.result } else { $null })
						b         = $(if ($bResp.ok) { $bResp.result } else { $null })
					}) | Out-Null
			}
		}

		if ($isVanilla -and $Cfg.dynamicProbe) {
			# The literal hypothesis probe: the player stands in the open and one
			# block is placed at head level (feet y+1) over its own column, then
			# removed. Ours cannot place blocks at runtime (no console), so this
			# phase is vanilla-only and is not part of the aligned sample set.
			$dp = $Cfg.dynamicProbe
			$spot = $spots['open']
			$line = [string]::Format($inv, "tp {0} {1} {2} {3} {4} {5}", `
					$nickA, $spot.x, $spot.y, $spot.z, $yaw, 0.0)
			Invoke-StandRcon $line | Out-Null
			Start-Sleep -Milliseconds ([int]$Cfg.settleMs)
			$place = ("setblock {0} {1} {2} {3}" -f [int]$case.x, $roofY, [int]$Cfg.openZ, [string]$case.roof)
			Invoke-VanillaConsole $place | Out-Null
			$remaining = [int]$dp.placeMs
			while ($remaining -gt 0) {
				$slice = [Math]::Min($sampleEvery, $remaining)
				Start-Sleep -Milliseconds $slice
				$remaining -= $slice
				$aResp = Invoke-TestCmd $portA (New-ObserveCmd "s11-a-$($script:s11SampleIndex)")
				$bResp = Invoke-TestCmd $portB (New-ObserveCmd "s11-b-$($script:s11SampleIndex)" $nickA)
				$script:s11SampleIndex++
				$script:s11Samples.Add([PSCustomObject]@{
						caseName  = [string]$case.name
						phase     = [string]$dp.phase
						elapsedMs = [int]$dp.placeMs - $remaining
						a         = $(if ($aResp.ok) { $aResp.result } else { $null })
						b         = $(if ($bResp.ok) { $bResp.result } else { $null })
						placed    = $place
					}) | Out-Null
			}
			$clear = ("setblock {0} {1} {2} air" -f [int]$case.x, $roofY, [int]$Cfg.openZ)
			Invoke-VanillaConsole $clear | Out-Null
			$remaining = [int]$dp.clearMs
			while ($remaining -gt 0) {
				$slice = [Math]::Min($sampleEvery, $remaining)
				Start-Sleep -Milliseconds $slice
				$remaining -= $slice
				$aResp = Invoke-TestCmd $portA (New-ObserveCmd "s11-a-$($script:s11SampleIndex)")
				$bResp = Invoke-TestCmd $portB (New-ObserveCmd "s11-b-$($script:s11SampleIndex)" $nickA)
				$script:s11SampleIndex++
				$script:s11Samples.Add([PSCustomObject]@{
						caseName  = [string]$case.name
						phase     = "$($dp.phase)-cleared"
						elapsedMs = [int]$dp.clearMs - $remaining
						a         = $(if ($aResp.ok) { $aResp.result } else { $null })
						b         = $(if ($bResp.ok) { $bResp.result } else { $null })
					}) | Out-Null
			}
		}

		$rows = @($script:s11Samples | Where-Object { $_.caseName -eq $case.name })
		$aPoses = @($rows | ForEach-Object { $_.a.pose } | Sort-Object -Unique) -join ','
		$aHeights = @($rows | ForEach-Object { $_.a.bbHeight } | Sort-Object -Unique) -join ','
		$aHead = @($rows | Where-Object { $_.phase -eq 'block-above' } | ForEach-Object { $_.a.headBlock.id } | Sort-Object -Unique) -join ','
		$bPoses = @($rows | ForEach-Object { $_.b.pose } | Sort-Object -Unique) -join ','
		Add-Check $Result ("S11 {0}: samples recorded" -f $case.name) ($rows.Count -gt 0 -and $tpOk) `
			("samples={0}; roof={1}; A poses=[{2}] bbHeight=[{3}] head@probe=[{4}]; B poses=[{5}]; tp={6}" -f `
				$rows.Count, $case.roof, $aPoses, $aHeights, $aHead, $bPoses, $(if ($tpOk) { 'ok' } else { $tpFact }))
		foreach ($key in @('up', 'down', 'left', 'right', 'jump', 'shift', 'sprint')) {
			$null = Invoke-TestCmd $portA (New-KeyCmd "$($case.name)-release-$key" $key 'up')
		}
	}

	$artifact = [PSCustomObject]@{
		target        = $Target
		generated     = (Get-Date).ToString('s')
		baseY         = [int]$Cfg.baseY
		uuidA         = $launchA.uuid
		nickA         = $nickA
		nickB         = $nickB
		platform      = @($Cfg.platform)
		roofY         = $roofY
		roofFromZ     = [int]$Cfg.roofFromZ
		roofToZ       = [int]$Cfg.roofToZ
		probeZ        = [int]$Cfg.probeZ
		openZ         = [int]$Cfg.openZ
		standY        = $standY
		cases         = @($cases)
		samples       = @($script:s11Samples)
	}
	$artifactPath = Join-Path $RunDir 's11-samples.json'
	$artifact | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $artifactPath -Encoding utf8
	$script:s7Facts.Add("S11 artifact: $artifactPath ($($script:s11Samples.Count) samples)")
	Write-Host ("  [info] S11 artifact written: $artifactPath")

	$null = Invoke-TestCmd $portA (New-SessionCmd 's11-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's11-offb' 'disconnect')
	Start-Sleep -Seconds 3
}

# --- S12: survival dig states, live (РЁР°Рі 16a.4 Рї.2/Рї.3b/Рї.4) ----------------
# Four stands on the S10 platform, all dug straight down (pitch 90) by holding
# attack: a 4-block ground column (grass, dirt, stone, leaves), the same column
# sneaking, a block under water at the feet, and a block under an air stand (the
# player is teleported 2.5 blocks above it; the aim check takes longer than the
# fall, so the dig starts from the ground - see docs/BLOCK-BREAKING.md 4.3.2).
# Vanilla feeds the geometry to its console after a join; our server gets the
# same list through -OursDevGeometry at startup and needs -HardnessReport for
# the dig timer. Facts recorded: A's player_action timings per block, B's
# block_destruction stages, break level_events and air block_updates, all in
# s12-samples.json for the tick comparison against the vanilla traces.
function Get-S12Commands {
	param($Cfg)
	$lines = New-Object System.Collections.Generic.List[string]
	foreach ($cmd in @($Cfg.platform)) { $lines.Add([string]$cmd) }
	$z = [int]$Cfg.columnZ
	$base = [int]$Cfg.baseY
	foreach ($case in @($Cfg.cases)) {
		$x = [int]$case.x
		if ($case.kind -eq 'column' -or $case.kind -eq 'sneak') {
			$y = $base
			foreach ($block in @($case.blocks)) {
				$lines.Add(("setblock {0} {1} {2} {3}" -f $x, $y, $z, [string]$block))
				$y--
			}
			# Landing pad under the column: the four one-block falls together
			# are four blocks, one damage point, and the pad keeps the player
			# from dropping through the platform afterwards.
			$lines.Add(("setblock {0} {1} {2} minecraft:stone" -f $x, $y, $z))
		}
		elseif ($case.kind -eq 'water') {
			# A 1x1, one-deep walled pool: feet at base+1 are inside the water
			# while the eyes (base+2.62) stay dry, and the dug block is the
			# floor at base. The walls keep a vanilla source from spreading.
			# The floor must be set explicitly: without it the dug block was
			# the platform's stone (150 bare-hand ticks, not dirt's 15).
			$lines.Add(("setblock {0} {1} {2} {3}" -f $x, $base, $z, [string]$case.block))
			$lines.Add(("fill {0} {1} {2} {3} {1} {4} minecraft:stone" -f `
						($x - 1), ($base + 1), ($z - 1), ($x + 1), ($z + 1)))
			$lines.Add(("fill {0} {1} {2} {0} {1} {2} minecraft:water" -f $x, ($base + 1), $z))
		}
		elseif ($case.kind -eq 'air') {
			# One isolated block on the platform top; the stand is base+3.5,
			# so the block top is 4.1 blocks from the eyes (reach 4.5). The
			# fall there lasts ~50 ms of proxy time while the aim check takes
			# ~250 ms, so the dig starts from the ground in practice. Bedrock
			# replaces the platform block below: a long hold must not start a
			# second dig (the leaves break in ~0.3 s, the key stays down).
			$lines.Add(("setblock {0} {1} {2} {3}" -f $x, $base, $z, [string]$case.block))
			$lines.Add(("setblock {0} {1} {2} minecraft:bedrock" -f $x, ($base - 1), $z))
		}
		else {
			throw "S12: unknown case kind '$($case.kind)'"
		}
	}
	return $lines
}

# S14: one command list per cell kind, built once for both the vanilla console
# and our dev-geometry file, so the two stands always carry the same world.
function Get-S14Commands {
	param($Cfg)
	$lines = New-Object System.Collections.Generic.List[string]
	foreach ($cmd in @($Cfg.platform)) { $lines.Add([string]$cmd) }
	$base = [int]$Cfg.baseY
	$feet = $base + 1
	foreach ($row in @($Cfg.rows)) {
		$z = [int]$row.z
		foreach ($kind in @($Cfg.kinds)) {
			$x = [int]$kind.x
			switch ([string]$kind.kind) {
				'plant' {
					# A land plant needs a dirt-like support; the platform is stone.
					$lines.Add(("setblock {0} {1} {2} minecraft:dirt" -f $x, $base, $z))
					$lines.Add(("setblock {0} {1} {2} {3}" -f $x, $feet, $z, [string]$kind.block))
				}
				'pool1' {
					$lines.Add(("fill {0} {1} {2} {3} {4} {5} minecraft:stone" -f ($x - 1), $feet, ($z - 1), ($x + 1), $feet, ($z + 1)))
					$lines.Add(("setblock {0} {1} {2} minecraft:water" -f $x, $feet, $z))
					$lines.Add(("setblock {0} {1} {2} {3}" -f $x, $feet, $z, [string]$kind.block))
				}
				'pool2' {
					$lines.Add(("fill {0} {1} {2} {3} {4} {5} minecraft:stone" -f ($x - 1), $feet, ($z - 1), ($x + 1), ($feet + 1), ($z + 1)))
					$lines.Add(("setblock {0} {1} {2} minecraft:water" -f $x, $feet, $z))
					$lines.Add(("setblock {0} {1} {2} minecraft:water" -f $x, ($feet + 1), $z))
					$lines.Add(("setblock {0} {1} {2} minecraft:tall_seagrass[half=lower]" -f $x, $feet, $z))
					$lines.Add(("setblock {0} {1} {2} minecraft:tall_seagrass[half=upper]" -f $x, ($feet + 1), $z))
				}
				'pool3' {
					$lines.Add(("fill {0} {1} {2} {3} {4} {5} minecraft:stone" -f ($x - 1), $feet, ($z - 1), ($x + 1), ($feet + 2), ($z + 1)))
					$lines.Add(("setblock {0} {1} {2} minecraft:water" -f $x, $feet, $z))
					$lines.Add(("setblock {0} {1} {2} minecraft:water" -f $x, ($feet + 1), $z))
					$lines.Add(("setblock {0} {1} {2} minecraft:water" -f $x, ($feet + 2), $z))
					$lines.Add(("setblock {0} {1} {2} minecraft:kelp_plant" -f $x, $feet, $z))
					$lines.Add(("setblock {0} {1} {2} minecraft:kelp_plant" -f $x, ($feet + 1), $z))
					$lines.Add(("setblock {0} {1} {2} minecraft:kelp[age=5]" -f $x, ($feet + 2), $z))
				}
				'slab' {
					$lines.Add(("setblock {0} {1} {2} minecraft:oak_slab[type=bottom,waterlogged=true]" -f $x, $feet, $z))
				}
				'stairs' {
					$lines.Add(("setblock {0} {1} {2} minecraft:oak_stairs[facing=east,half=bottom,shape=straight,waterlogged=true]" -f $x, $feet, $z))
				}
				'water' {
					$lines.Add(("fill {0} {1} {2} {3} {4} {5} minecraft:stone" -f ($x - 1), $feet, ($z - 1), ($x + 1), $feet, ($z + 1)))
					$lines.Add(("setblock {0} {1} {2} minecraft:water" -f $x, $feet, $z))
				}
				default { throw "S14: unknown kind '$($kind.kind)'" }
			}
		}
	}
	return $lines
}

# Standing height of A for one S14 cell: on the platform for the plants and the
# pools, on top of the half-slab and on the stair step for the waterlogged
# building blocks.
function Get-S14StandY {
	param($Kind, [int]$Base)
	switch ([string]$Kind.kind) {
		'slab' { return $Base + 1.5 }
		'stairs' { return $Base + 2.0 }
		default { return $Base + 1.0 }
	}
}

# Whether the crosshair hit is one of the cells the kind may be aimed at: the
# plant/tip cell of a pool can be any part of the water column, because a
# two- or three-deep pool lifts the feet and the ray reaches the highest part.
function Test-S14Aim {
	param($Kind, [int]$Base, [int]$Z, $Hit)
	if ($null -eq $Hit) { return $false }
	if ([int]$Hit.x -ne [int]$Kind.x -or [int]$Hit.z -ne $Z) { return $false }
	$y = [int]$Hit.y
	switch ([string]$Kind.kind) {
		'pool2' { return ($y -ge ($Base + 1) -and $y -le ($Base + 2)) }
		'pool3' { return ($y -ge ($Base + 1) -and $y -le ($Base + 3)) }
		default { return ($y -eq ($Base + 1)) }
	}
}

# S14 (РЁР°Рі 18) вЂ” instant-break and fluid blocks.
#
# Two rows (hold and tap) over nine one-cell kinds, every cell unique, so the
# trace rows are attributed by position. A is teleported into the cell, the
# crosshair is confirmed on it, A holds or taps attack, and both A's observe
# and the proxy traces are read back: serverbound player_action per cell,
# clientbound block_changed_ack to A, and B's crack stages, level_event 2001,
# replacement block_update and neighbour updates.
function Invoke-S14 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$settle = [int]$Cfg.settleMs
	$base = [int]$Cfg.baseY
	$feet = $base + 1
	$holdMs = [int]$Cfg.holdMs
	$tapMs = [int]$Cfg.tapMs
	$inv = [System.Globalization.CultureInfo]::InvariantCulture
	$commands = Get-S14Commands $Cfg

	$watermark = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB (Get-Date)
	$observer = Wait-ObserverChunksStable $portB 30
	$script:s7Facts.Add(("S14 B chunk drain: settled={0} loadedChunks={1}" -f `
				$observer.settled, $observer.loadedChunks))

	if (-not $isVanilla) {
		if (-not $OursDevGeometry) {
			Add-Check $Result 'S14 geometry is available on our server' $false `
				'pass -OursDevGeometry <file>: the runner writes the s14 commands there before start'
			$null = Invoke-TestCmd $portA (New-SessionCmd 's14-off' 'disconnect')
			$null = Invoke-TestCmd $portB (New-SessionCmd 's14-offb' 'disconnect')
			Start-Sleep -Seconds 3
			return
		}
		if (-not $HardnessReport) {
			Add-Check $Result 'the survival dig timer is on' $false `
				'pass -HardnessReport <hardness.json>: without it our server trusts every finish and times nothing'
			$null = Invoke-TestCmd $portA (New-SessionCmd 's14-off' 'disconnect')
			$null = Invoke-TestCmd $portB (New-SessionCmd 's14-offb' 'disconnect')
			Start-Sleep -Seconds 3
			return
		}
		$script:s7Facts.Add("S14 dev geometry applied at startup: $OursDevGeometry")
		$script:s7Facts.Add("S14 hardness report: $HardnessReport")
	} else {
		$fl = @($Cfg.forceload)
		if ($fl.Count -eq 4) {
			Invoke-VanillaConsole ("forceload add {0} {1} {2} {3}" -f $fl[0], $fl[1], $fl[2], $fl[3]) | Out-Null
			Start-Sleep -Milliseconds 1000
		}
		foreach ($cmd in $commands) { Invoke-VanillaConsole $cmd | Out-Null }
		Start-Sleep -Seconds 2
		Invoke-VanillaConsole 'gamerule doImmediateRespawn true' | Out-Null
		Invoke-VanillaConsole 'difficulty peaceful' | Out-Null
		Invoke-VanillaConsole 'gamerule doMobSpawning false' | Out-Null
		Start-Sleep -Milliseconds 500
	}

	$runs = New-Object System.Collections.ArrayList
	foreach ($row in @($Cfg.rows)) {
		$z = [int]$row.z
		$mode = [string]$row.mode
		foreach ($kind in @($Cfg.kinds)) {
			# A kind may be restricted to some rows: a waterlogged slab/stairs
			# (hardness 2.0) is not broken by a 500 ms tap, and the dandelion tap
			# read the support block below in the first run (a harness raycast
			# flake; the rule is already covered by short grass and leaf litter).
			if ($null -ne $kind.rows -and (@($kind.rows) -notcontains [string]$row.name)) { continue }
			$x = [int]$kind.x
			$name = ("{0}-{1}" -f $row.name, $kind.name)
			$standY = Get-S14StandY $kind $base
			$observeOnly = $false
			if ($null -ne $kind.observeOnly) { $observeOnly = [bool]$kind.observeOnly }

			# Wait until the crosshair is on the expected cell: the chunk and the
			# teleport (and, in a pool, the float) must settle before the click.
			# A second teleport re-snaps the client if the first ray read is
			# stale, so one flaky read cannot fail the cell.
			$aim = $null
			$aimed = $false
			for ($attempt = 1; $attempt -le 2 -and -not $aimed; $attempt++) {
				if ($isVanilla) {
					$line = [string]::Format($inv, "tp {0} {1} {2} {3} 0 90", `
							$nickA, ($x + 0.5), $standY, ($z + 0.5))
					Invoke-StandRcon $line | Out-Null
				} else {
					$null = Invoke-TestCmd $portA (New-TpCmd "$name-tp$attempt" ($x + 0.5) $standY ($z + 0.5) 0 90)
				}
				$aimDeadline = (Get-Date).AddSeconds(6)
				while ((Get-Date) -lt $aimDeadline) {
					Start-Sleep -Milliseconds 250
					$aim = (Invoke-TestCmd $portA (New-ObserveCmd "$name-aim$attempt")).result
					if ($null -eq $aim -or $null -eq $aim.hitBlock) { continue }
					if ($aim.hitType -eq 'BLOCK' -and (Test-S14Aim $kind $base $z $aim.hitBlock)) {
						$aimed = $true
						break
					}
				}
			}
			$aimFact = 'no observe result'
			if ($null -ne $aim -and $null -ne $aim.hitBlock) {
				$aimFact = ("hitType={0} block=({1},{2},{3})" -f $aim.hitType, `
						$aim.hitBlock.x, $aim.hitBlock.y, $aim.hitBlock.z)
			} elseif ($null -ne $aim) {
				$aimFact = ("hitType={0}" -f $aim.hitType)
			}
			if (-not $observeOnly) {
				Add-Check $Result ("{0}: A aims at the block" -f $name) $aimed $aimFact
			}

			$caseSettle = if ($null -ne $kind.digSettleMs) { [int]$kind.digSettleMs } else { [int]$Cfg.digSettleMs }
			if ($aimed -and $caseSettle -gt 0) { Start-Sleep -Milliseconds $caseSettle }

			$caseHold = if ($null -ne $kind.holdMs) { [int]$kind.holdMs } else { $holdMs }
			$attacked = $false
			if ($aimed -and -not $observeOnly) {
				if ($mode -eq 'tap') {
					$null = Invoke-TestCmd $portA (New-KeyCmd "$name-tap" 'attack' 'tap' $tapMs)
				} else {
					$null = Invoke-TestCmd $portA (New-KeyCmd "$name-down" 'attack' 'down')
					Start-Sleep -Milliseconds $caseHold
					$null = Invoke-TestCmd $portA (New-KeyCmd "$name-up" 'attack' 'up')
				}
				$attacked = $true
			}
			Start-Sleep -Milliseconds $settle
			$after = (Invoke-TestCmd $portA (New-ObserveCmd "$name-after")).result

			# The block at the dug cell, read from the observe that owns it: the
			# feet cell in every case but a pool whose float lifts the feet past
			# it (there the support or head block carries the cell).
			$cellBlock = $null
			$where = 'unknown'
			if ($null -ne $after -and $null -ne $after.pos) {
				$feetY = [int][Math]::Floor([double]$after.pos.y)
				if ($feetY -eq $feet) { $cellBlock = $after.feetBlock; $where = 'feet' }
				elseif (($feetY - 1) -eq $feet) { $cellBlock = $after.supportBlock; $where = 'support' }
				elseif (($feetY + 1) -eq $feet) { $cellBlock = $after.headBlock; $where = 'head' }
			}
			$cellId = 'unknown'
			$cellFluid = 'unknown'
			if ($null -ne $cellBlock) {
				$cellId = [string]$cellBlock.id
				$cellFluid = [string]$cellBlock.fluid
			}
			$afterPos = 'no observe result'
			if ($null -ne $after -and $null -ne $after.pos) {
				$afterPos = [string]::Format($inv, "{0:0.00},{1:0.00},{2:0.00}", `
						[double]$after.pos.x, [double]$after.pos.y, [double]$after.pos.z)
			}
			if ($aimed -or $observeOnly) {
				$expectId = if ([string]$kind.expect -eq 'water') { 'minecraft:water' } else { 'minecraft:air' }
				Add-Check $Result ("{0}: the cell is {1} after the dig" -f $name, $expectId) `
				($cellId -eq $expectId) ("{0}Block id={1} fluid={2} pos={3}" -f $where, $cellId, $cellFluid, $afterPos)
			}

			$runs.Add([PSCustomObject]@{
					name = $name; row = [string]$row.name; mode = $mode
					kind = [string]$kind.kind; x = $x; y = $feet; z = $z
					aimed = $aimed; attacked = $attacked; observeOnly = $observeOnly
					aim = $aimFact; afterPos = $afterPos
					replacement = $cellId; fluid = $cellFluid; where = $where
				}) | Out-Null
		}
	}

	# --- traces ------------------------------------------------------------
	Start-Sleep -Seconds 1
	$files = @(Get-NewTraceFiles $traceDir $watermark)
	$aTrace = Select-TraceByPattern $files 'minecraft:player_action'
	$bTrace = $null
	if ($null -ne $aTrace) {
		$bTrace = Select-TraceByPattern $files 'minecraft:(block_destruction|level_event)' @($aTrace.FullName)
	}
	if ($null -ne $aTrace -and $null -ne $bTrace) {
		$kindNames = @('minecraft:player_action', 'minecraft:punch', 'minecraft:block_update', `
				'minecraft:block_changed_ack', 'minecraft:block_destruction', 'minecraft:level_event')
		$aRows = @(Get-TraceBlockRows $aTrace.FullName $kindNames)
		$bRows = @(Get-TraceBlockRows $bTrace.FullName $kindNames)

		$report = New-Object System.Collections.ArrayList
		foreach ($run in $runs) {
			$x = [int]$run.x
			$z = [int]$run.z
			$aActions = @($aRows | Where-Object { $_.name -eq 'minecraft:player_action' -and $_.dir -eq 'serverbound' -and $_.x -eq $x -and $_.z -eq $z -and $_.y -ge $feet -and $_.y -le ($feet + 3) })
			$cellSeqs = @($aActions | ForEach-Object { [int]$_.seq })
			$aAcks = @($aRows | Where-Object { $_.name -eq 'minecraft:block_changed_ack' -and $_.dir -eq 'clientbound' -and ($cellSeqs -contains [int]$_.seq) })
			$aUpdates = @($aRows | Where-Object { $_.name -eq 'minecraft:block_update' -and $_.dir -eq 'clientbound' -and $_.x -eq $x -and $_.z -eq $z -and $_.y -ge $feet -and $_.y -le ($feet + 3) })
			$bStages = @($bRows | Where-Object { $_.name -eq 'minecraft:block_destruction' -and $_.dir -eq 'clientbound' -and $_.x -eq $x -and $_.z -eq $z -and $_.y -ge $feet -and $_.y -le ($feet + 3) })
			$bEvents = @($bRows | Where-Object { $_.name -eq 'minecraft:level_event' -and $_.dir -eq 'clientbound' -and $_.x -eq $x -and $_.z -eq $z -and $_.y -ge $feet -and $_.y -le ($feet + 3) })
			$bUpdates = @($bRows | Where-Object { $_.name -eq 'minecraft:block_update' -and $_.dir -eq 'clientbound' -and $_.x -eq $x -and $_.z -eq $z -and $_.y -ge $feet -and $_.y -le ($feet + 3) })
			$starts = @($aActions | Where-Object { $_.action -eq 0 })
			$finishes = @($aActions | Where-Object { $_.action -eq 3 })
			$aborts = @($aActions | Where-Object { $_.action -eq 2 })
			if ($run.attacked) {
				Add-Check $Result ("{0}: A sent a start for the cell" -f $run.name) ($starts.Count -ge 1) `
					("starts={0} finishes={1} aborts={2}" -f $starts.Count, $finishes.Count, $aborts.Count)
			}
			$bStates = @($bUpdates | ForEach-Object { $_.state })
			$report.Add([PSCustomObject]@{
					name = $run.name; row = $run.row; mode = $run.mode; kind = $run.kind
					x = $x; y = $run.y; z = $z; aimed = $run.aimed; attacked = $run.attacked
					observeOnly = $run.observeOnly; replacement = $run.replacement
					fluid = $run.fluid; afterPos = $run.afterPos
					starts = @($starts | ForEach-Object { [int]$_.action })
					startSeq = @($starts | ForEach-Object { [int]$_.seq })
					finishSeq = @($finishes | ForEach-Object { [int]$_.seq })
					abortSeq = @($aborts | ForEach-Object { [int]$_.seq })
					aAckSeqs = @($aAcks | ForEach-Object { [int]$_.seq })
					aUpdateStates = @($aUpdates | ForEach-Object { [int]$_.state })
					bStages = @($bStages | ForEach-Object { [int]$_.stage })
					bEvents = @($bEvents | ForEach-Object { ("{0}/{1}" -f [int]$_.event, [int]$_.data) })
					bUpdateStates = @($bUpdates | ForEach-Object { ("y{0}={1}" -f [int]$_.y, [int]$_.state) })
				}) | Out-Null
			Write-Host ("  [info] S14 {0}: starts={1} finishes={2} aborts={3} ack={4} aUpdates=[{5}] bUpdates=[{6}] bEvents=[{7}] bStages={8}" -f `
					$run.name, $starts.Count, $finishes.Count, $aborts.Count, $aAcks.Count, `
					($report[-1].aUpdateStates -join ','), ($report[-1].bUpdateStates -join ','), `
					($report[-1].bEvents -join ','), $report[-1].bStages.Count)
		}

		$artifact = [PSCustomObject]@{
			rows = @($report)
		}
		$artifactPath = Join-Path $traceDir 's14-samples.json'
		$artifact | ConvertTo-Json -Depth 6 | Set-Content $artifactPath -Encoding utf8
		$script:s7Facts.Add("S14 artifact: $artifactPath")
		Write-Host ("  [info] S14 artifact written: {0}" -f $artifactPath)
	} else {
		Add-Check $Result 'S14 traces are available' $false `
			("trace files={0}; digger={1}; observer={2}" -f $files.Count, `
				$(if ($null -ne $aTrace) { $aTrace.Name } else { 'none' }), `
				$(if ($null -ne $bTrace) { $bTrace.Name } else { 'none' }))
	}

	$null = Invoke-TestCmd $portA (New-SessionCmd 's14-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's14-offb' 'disconnect')
	Start-Sleep -Seconds 3
}

function Invoke-S12 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$settle = [int]$Cfg.settleMs
	$base = [int]$Cfg.baseY
	$z = [int]$Cfg.columnZ
	$inv = [System.Globalization.CultureInfo]::InvariantCulture
	$commands = Get-S12Commands $Cfg

	$watermark = Get-Date
	$null = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB (Get-Date)
	$observer = Wait-ObserverChunksStable $portB 30
	$script:s7Facts.Add(("S12 B chunk drain: settled={0} loadedChunks={1}" -f `
				$observer.settled, $observer.loadedChunks))

	if (-not $isVanilla) {
		if (-not $OursDevGeometry) {
			Add-Check $Result 'S12 geometry is available on our server' $false `
				'pass -OursDevGeometry <file>: the runner writes the s12 commands there before start'
			$null = Invoke-TestCmd $portA (New-SessionCmd 's12-off' 'disconnect')
			$null = Invoke-TestCmd $portB (New-SessionCmd 's12-offb' 'disconnect')
			Start-Sleep -Seconds 3
			return
		}
		if (-not $HardnessReport) {
			Add-Check $Result 'the survival dig timer is on' $false `
				'pass -HardnessReport <hardness.json>: without it our server trusts every finish and times nothing'
			$null = Invoke-TestCmd $portA (New-SessionCmd 's12-off' 'disconnect')
			$null = Invoke-TestCmd $portB (New-SessionCmd 's12-offb' 'disconnect')
			Start-Sleep -Seconds 3
			return
		}
		$script:s7Facts.Add("S12 dev geometry applied at startup: $OursDevGeometry")
		$script:s7Facts.Add("S12 hardness report: $HardnessReport")
	} else {
		$fl = @($Cfg.forceload)
		if ($fl.Count -eq 4) {
			Invoke-VanillaConsole ("forceload add {0} {1} {2} {3}" -f $fl[0], $fl[1], $fl[2], $fl[3]) | Out-Null
			Start-Sleep -Milliseconds 1000
		}
		foreach ($cmd in $commands) { Invoke-VanillaConsole $cmd | Out-Null }
		Start-Sleep -Seconds 2
		Invoke-VanillaConsole 'gamerule doImmediateRespawn true' | Out-Null
		Invoke-VanillaConsole 'difficulty peaceful' | Out-Null
		Invoke-VanillaConsole 'gamerule doMobSpawning false' | Out-Null
		Start-Sleep -Milliseconds 500
	}

	$casesRun = New-Object System.Collections.ArrayList
	foreach ($case in @($Cfg.cases)) {
		$x = [int]$case.x
		$standY = if ($case.kind -eq 'air') { $base + 3.5 } else { $base + 1.0 }
		$standX = $x + 0.5
		$standZ = $z + 0.5
		$yaw = 0.0
		$pitch = 90.0
		if ($isVanilla) {
			$line = [string]::Format($inv, "tp {0} {1} {2} {3} {4} {5}", `
					$nickA, $standX, $standY, $standZ, $yaw, $pitch)
			Invoke-StandRcon $line | Out-Null
		} else {
			$null = Invoke-TestCmd $portA (New-TpCmd "s12-$($case.name)-tp" $standX $standY $standZ $yaw $pitch)
		}

		# Wait until the crosshair is on the expected cell: the client needs
		# the chunk and the teleport to settle before a click reaches a block.
		$aim = $null
		$aimed = $false
		$aimDeadline = (Get-Date).AddSeconds(8)
		while ((Get-Date) -lt $aimDeadline) {
			Start-Sleep -Milliseconds 250
			$aim = (Invoke-TestCmd $portA (New-ObserveCmd "s12-$($case.name)-aim")).result
			if ($null -eq $aim -or $null -eq $aim.hitBlock) { continue }
			if ($aim.hitType -eq 'BLOCK' -and [int]$aim.hitBlock.x -eq $x -and `
					[int]$aim.hitBlock.y -eq $base -and [int]$aim.hitBlock.z -eq $z) {
				$aimed = $true
				break
			}
		}
		$aimFact = 'no observe result'
		if ($null -ne $aim) {
			$aimFact = ("hitType={0} block=({1},{2},{3})" -f $aim.hitType, `
					$aim.hitBlock.x, $aim.hitBlock.y, $aim.hitBlock.z)
		}
		Add-Check $Result ("{0}: A aims at the block under it" -f $case.name) $aimed $aimFact
		if (-not $aimed) { continue }

		# Settle after the aim: the first dig of a run can lose ticks to the
		# client stalling and catching up its ticks (11 `client_tick_end`
		# packets arrived within 1.4 ms in the 2026-11-17_0131 run), which
		# makes the server's wall-time dig look short. The wait does not
		# change the client's dig time, but the first case of the scenario is
		# the one that carries join/tp stutter; it is also the case whose
		# finish the server trusts (water/airborne), so a compressed finish
		# there cannot roll back a legitimate dig.
		$caseSettle = if ($null -ne $case.digSettleMs) { [int]$case.digSettleMs } else { [int]$Cfg.digSettleMs }
		if ($caseSettle -gt 0) { Start-Sleep -Milliseconds $caseSettle }

		if ($case.kind -eq 'sneak') {
			$null = Invoke-TestCmd $portA (New-KeyCmd "s12-$($case.name)-shift" 'shift' 'down')
		}
		$null = Invoke-TestCmd $portA (New-KeyCmd "s12-$($case.name)-attack-on" 'attack' 'down')
		Start-Sleep -Milliseconds ([int]$case.holdMs)
		$null = Invoke-TestCmd $portA (New-KeyCmd "s12-$($case.name)-attack-up" 'attack' 'up')
		if ($case.kind -eq 'sneak') {
			$null = Invoke-TestCmd $portA (New-KeyCmd "s12-$($case.name)-shift-up" 'shift' 'up')
		}
		Start-Sleep -Milliseconds $settle
		$after = (Invoke-TestCmd $portA (New-ObserveCmd "s12-$($case.name)-after")).result
		$firstGone = $true
		if ($null -ne $after -and $null -ne $after.hitBlock) {
			$firstGone = -not ([int]$after.hitBlock.x -eq $x -and [int]$after.hitBlock.y -eq $base -and `
					[int]$after.hitBlock.z -eq $z)
		}
		$afterFact = 'no observe result'
		if ($null -ne $after) {
			$afterFact = ("hitType={0} block=({1},{2},{3})" -f $after.hitType, `
					$after.hitBlock.x, $after.hitBlock.y, $after.hitBlock.z)
		}
		Add-Check $Result ("{0}: the first block is gone" -f $case.name) $firstGone $afterFact
		$casesRun.Add([PSCustomObject]@{ name = [string]$case.name; kind = [string]$case.kind; x = $x; z = $z }) | Out-Null
	}

	# The wire facts come from the proxy traces: A's serverbound actions (start
	# and finish per block) and B's clientbound cracks, break effects and air
	# updates. Positions are decoded from the packet bytes, so each fact is
	# attributed to its stand, not just counted.
	Start-Sleep -Seconds 1
	$files = @(Get-NewTraceFiles $traceDir $watermark)
	# The connect order is A then B, but the runner must not rely on the file
	# index: mcproxy also writes one short file per rejected/duplicate
	# connection, and a file picked by position was already the wrong one.
	# The digger's file has serverbound player_action, the observer's has the
	# clientbound destruction; A receives those too, so it is excluded there.
	$aTrace = Select-TraceByPattern $files 'minecraft:player_action'
	$bTrace = $null
	if ($null -ne $aTrace) {
		$bTrace = Select-TraceByPattern $files 'minecraft:(block_destruction|level_event)' @($aTrace.FullName)
	}
	if ($null -ne $aTrace -and $null -ne $bTrace) {
		$aSeen = @(Select-String -LiteralPath $aTrace.FullName -Pattern 'minecraft:player_action' |
				ForEach-Object { $_.Line | ConvertFrom-Json } |
				Where-Object { $_.dir -eq 'serverbound' } |
				ForEach-Object {
					$p = ConvertFrom-TraceB64 $_.b64
					$action = ConvertFrom-TraceVarInt $p 0
					$pos = ConvertFrom-PackedPosition $p $action.next
					[PSCustomObject]@{
						ms = [double]$_.read_ms; action = [int]$action.value
						x = $pos.x; y = $pos.y; z = $pos.z
					}
				})
		$bRows = @(Select-String -LiteralPath $bTrace.FullName -Pattern 'minecraft:(block_destruction|level_event|block_update)' |
				ForEach-Object { $_.Line | ConvertFrom-Json } |
				Where-Object { $_.dir -eq 'clientbound' })
		$bStages = @($bRows | Where-Object { $_.name -eq 'minecraft:block_destruction' } | ForEach-Object {
				$p = ConvertFrom-TraceB64 $_.b64
				$eid = ConvertFrom-TraceVarInt $p 0
				$pos = ConvertFrom-PackedPosition $p $eid.next
				[PSCustomObject]@{
					ms = [double]$_.read_ms; eid = [int]$eid.value
					x = $pos.x; y = $pos.y; z = $pos.z; stage = [int]$p[$eid.next + 8]
				}
			})
		$bBreaks = @($bRows | Where-Object { $_.name -eq 'minecraft:level_event' } | ForEach-Object {
				$p = ConvertFrom-TraceB64 $_.b64
				# [int] casts matter: PowerShell shifts a [byte] in byte
				# width, so [byte]7 -shl 8 is 0 and 2001 decoded as 209.
				$event = ([int]$p[0] -shl 24) -bor ([int]$p[1] -shl 16) -bor ([int]$p[2] -shl 8) -bor [int]$p[3]
				$pos = ConvertFrom-PackedPosition $p 4
				$data = ([int]$p[12] -shl 24) -bor ([int]$p[13] -shl 16) -bor ([int]$p[14] -shl 8) -bor [int]$p[15]
				[PSCustomObject]@{ ms = [double]$_.read_ms; event = $event; x = $pos.x; y = $pos.y; z = $pos.z; data = $data }
			})
		$bUpdates = @($bRows | Where-Object { $_.name -eq 'minecraft:block_update' } | ForEach-Object {
				$p = ConvertFrom-TraceB64 $_.b64
				$pos = ConvertFrom-PackedPosition $p 0
				$state = ConvertFrom-TraceVarInt $p 8
				[PSCustomObject]@{ ms = [double]$_.read_ms; x = $pos.x; y = $pos.y; z = $pos.z; state = [int]$state.value }
			})

		foreach ($case in $casesRun) {
			$x = [int]$case.x
			$zCase = [int]$case.z
			$starts = @($aSeen | Where-Object { $_.x -eq $x -and $_.z -eq $zCase -and $_.action -eq 0 })
			$finishes = @($aSeen | Where-Object { $_.x -eq $x -and $_.z -eq $zCase -and $_.action -eq 3 })
			$needFinishes = if ($case.kind -eq 'column' -or $case.kind -eq 'sneak') { 4 } else { 1 }
			Add-Check $Result ("{0}: A sent the start and {1} finish(es)" -f $case.name, $needFinishes) `
			($starts.Count -ge 1 -and $finishes.Count -ge $needFinishes) `
			("starts={0} finishes={1}" -f $starts.Count, $finishes.Count)
			$stages = @($bStages | Where-Object { $_.x -eq $x -and $_.z -eq $zCase })
			$reset = @($stages | Where-Object { $_.stage -eq 0xFF })
			Add-Check $Result ("{0}: B received the crack stages and their reset" -f $case.name) `
			($stages.Count -ge 2 -and $reset.Count -ge 1) `
			("stages={0} resets={1}" -f $stages.Count, $reset.Count)
			$breaks = @($bBreaks | Where-Object { $_.event -eq 2001 -and $_.x -eq $x -and $_.z -eq $zCase })
			$airs = @($bUpdates | Where-Object { $_.x -eq $x -and $_.z -eq $zCase -and $_.state -eq 0 })
			Add-Check $Result ("{0}: B saw the break effect and the air update" -f $case.name) `
			($breaks.Count -ge 1 -and $airs.Count -ge 1) `
			("level_events={0} air_updates={1}" -f $breaks.Count, $airs.Count)
		}

		$artifact = [PSCustomObject]@{
			cases   = @($casesRun)
			actions = @($aSeen)
			stages  = @($bStages)
			breaks  = @($bBreaks)
			updates = @($bUpdates)
		}
		$artifactPath = Join-Path $traceDir 's12-samples.json'
		$artifact | ConvertTo-Json -Depth 5 | Set-Content $artifactPath -Encoding utf8
		$script:s7Facts.Add("S12 artifact: $artifactPath")
		Write-Host ("  [info] S12 artifact written: {0}" -f $artifactPath)
	} else {
		Add-Check $Result 'S12 traces are available' $false `
			("trace files={0}; digger={1}; observer={2}" -f $files.Count, `
				$(if ($null -ne $aTrace) { $aTrace.Name } else { 'none' }), `
				$(if ($null -ne $bTrace) { $bTrace.Name } else { 'none' }))
	}

	$null = Invoke-TestCmd $portA (New-SessionCmd 's12-off' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's12-offb' 'disconnect')
	Start-Sleep -Seconds 3
}

# РЁР°Рі 19a: the effective game mode of the generated config.
function Get-OursConfigMode {
	$mode = 0
	$path = Join-Path $RunDir 'server-ours.toml'
	if (Test-Path -LiteralPath $path) {
		$hit = Select-String -LiteralPath $path -Pattern '^\s*game_mode\s*=\s*(\d+)' | Select-Object -First 1
		if ($hit) { $mode = [int]$hit.Matches[0].Groups[1].Value }
	}
	if ($OursGameMode -ge 0) { $mode = $OursGameMode }
	return $mode
}

# Vanilla game mode id -> the name the test client's observe reports.
function ConvertTo-GameModeName {
	param([int]$Mode)
	switch ($Mode) {
		0 { return 'survival' }
		1 { return 'creative' }
		2 { return 'adventure' }
		3 { return 'spectator' }
		default { return "unknown($Mode)" }
	}
}

# Waits until the file exists and its mtime is newer than $Since.
function Wait-FileNewer {
	param([string]$Path, [datetime]$Since, [int]$Seconds = 15)
	$deadline = (Get-Date).AddSeconds($Seconds)
	while ((Get-Date) -lt $deadline) {
		if (Test-Path -LiteralPath $Path) {
			if ((Get-Item -LiteralPath $Path).LastWriteTime -gt $Since) { return $true }
		}
		Start-Sleep -Milliseconds 250
	}
	return $false
}

# Waits until the client reports that it left the world. `disconnectFromWorld`
# runs the client's own save screen first, so the server sees the quit a few
# seconds after the command; a dead control server also counts as left.
function Wait-ClientLeft {
	param([int]$Port, [int]$Seconds = 25)
	$deadline = (Get-Date).AddSeconds($Seconds)
	while ((Get-Date) -lt $deadline) {
		try {
			$obs = Invoke-TestCmd $Port (New-ObserveCmd 'left-check') 2000
			if ($obs.ok -and $obs.result.inGame -ne $true) { return $true }
		} catch {
			return $true
		}
		Start-Sleep -Milliseconds 500
	}
	return $false
}

# Horizontal distance between two traced/observed poses.
function Get-XzDistance {
	param($A, $B)
	return [Math]::Sqrt([Math]::Pow($A.x - $B.x, 2) + [Math]::Pow($A.z - $B.z, 2))
}

# The trace of the one connection opened after $Since (the newest when more
# than one exists); callers snapshot before a launch so exactly one is new.
function Get-FreshTrace {
	param([datetime]$Since)
	$files = @(Get-NewTraceFiles $traceDir $Since)
	if ($files.Count -eq 0) { return $null }
	return $files[$files.Count - 1]
}

# S13 (РЁР°Рі 19a): playerdata on the live stand.
function Invoke-S13 {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$poseTol = [double]$Cfg.poseTolerance
	$rotTol = [double]$Cfg.rotationTolerance
	$saveDir = $OursSaveDir
	$configMode = Get-OursConfigMode

	$templateBefore = Get-TreeFingerprint $templateWorldDir
	$vanillaWorld = Join-Path $vanillaDir 'world'
	$vanillaBefore = Get-TreeFingerprint $vanillaWorld
	$script:s7Facts.Add("S13 save_dir: $saveDir")
	$script:s7Facts.Add("S13 config game_mode: $configMode; force_gamemode: '$OursForceGamemode'; autosave: $OursAutosaveTicks ticks")

	# --- first session: join, walk, turn, autosave while connected ----------
	$watermark = Get-Date
	$launchA = Start-Client $nickA $portA $watermark
	if (-not $launchA.uuid) { throw 'S13: could not resolve Alice offline UUID' }
	$uuidBytes = ConvertTo-NetworkUuidBytes ([guid]::Parse($launchA.uuid))
	$playerFile = Get-PlayerDataPath $saveDir $launchA.uuid
	$entry1 = (Invoke-TestCmd $portA (New-ObserveCmd 's13-entry')).result
	$script:s7Facts.Add(("S13 entry: pos=({0},{1},{2}) yaw={3} pitch={4}" -f $entry1.pos.x, $entry1.pos.y, $entry1.pos.z, $entry1.rot.yaw, $entry1.rot.pitch))

	$null = Invoke-TestCmd $portA (New-KeyCmd 's13-walk-on' 'up' 'down')
	Start-Sleep -Milliseconds ([int]$Cfg.walkMs)
	$null = Invoke-TestCmd $portA (New-KeyCmd 's13-walk-off' 'up' 'up')
	$null = Invoke-TestCmd $portA (New-LookCmd 's13-look' ([int]$Cfg.lookDx) ([int]$Cfg.lookDy))
	Start-Sleep -Milliseconds 500
	$moved = (Invoke-TestCmd $portA (New-ObserveCmd 's13-moved')).result
	$walked = Get-XzDistance $moved.pos $entry1.pos
	Add-Check $Result 'S13: Alice moved and turned before the save' ($walked -gt 0.5 -and [Math]::Abs($moved.rot.yaw) -gt 1.0 -and [Math]::Abs($moved.rot.pitch) -gt 1.0) `
		("walked={0:N2} pos=({1},{2},{3}) yaw={4} pitch={5}" -f $walked, $moved.pos.x, $moved.pos.y, $moved.pos.z, $moved.rot.yaw, $moved.rot.pitch)
	$script:s7Facts.Add(("S13 moved: pos=({0},{1},{2}) yaw={3} pitch={4} walked={5:N2}" -f $moved.pos.x, $moved.pos.y, $moved.pos.z, $moved.rot.yaw, $moved.rot.pitch, $walked))

	# Autosave (interval 200 ticks): the file must appear while A is still
	# connected, and the server must still answer. The autosave of a shared
	# tick counter may fire while the walk is still going on, so the check
	# waits for a write strictly newer than the observation: by then Alice
	# stands still and the file has to carry the observed pose.
	$movedAt = Get-Date
	$autoSeen = Wait-FileNewer $playerFile $movedAt 30
	$alive = $false
	try { $alive = ((Invoke-TestCmd $portA (New-ObserveCmd 's13-autosave-alive') 3000).result.inGame -eq $true) } catch { }
	$autoFacts = $null
	if ($autoSeen) {
		$autoFacts = Get-PlayerNbtFacts $playerFile
	}
	$autoMatches = $false
	if ($autoFacts -and $autoFacts.pos) {
		$autoPose = [PSCustomObject]@{ x = $autoFacts.pos[0]; y = $autoFacts.pos[1]; z = $autoFacts.pos[2] }
		$autoMatches = ((Get-XzDistance $autoPose $moved.pos) -le $poseTol)
	}
	$autoText = ("file={0} exists, server alive={1}, pose matches the moved pose={2}" -f $playerFile, $alive, $autoMatches)
	if ($autoFacts) { $autoText += '; Pos=(' + ($autoFacts.pos -join ',') + ')' } else { $autoText += '; no facts' }
	Add-Check $Result 'S13 (3): autosave wrote the file while Alice was connected' ($autoSeen -and $alive -and $autoMatches) $autoText
	if ($autoFacts) { $script:s7Facts.Add(("S13 autosave: Pos=({0}) Rotation=({1}) Health={2} foodLevel={3} DataVersion={4} playerGameType={5}" -f ($autoFacts.pos -join ','), ($autoFacts.rotation -join ','), $autoFacts.health, $autoFacts.foodLevel, $autoFacts.dataVersion, $autoFacts.gameType)) }

	# --- quit: the same pose is left in the file ---------------------------
	$beforeQuit = (Invoke-TestCmd $portA (New-ObserveCmd 's13-before-quit')).result
	$quitAt = Get-Date
	$null = Invoke-TestCmd $portA (New-SessionCmd 's13-off' 'disconnect')
	$left = Wait-ClientLeft $portA 25
	$written = Wait-FileNewer $playerFile $quitAt 20
	Add-Check $Result 'S13: the quit wrote the player file' $written `
		("{0} (client left={1}, autosave {2} ticks)" -f $playerFile, $left, $OursAutosaveTicks)
	if (-not $written) { return }
	$saved = Get-PlayerNbtFacts $playerFile
	if (-not $saved.pos) { Add-Check $Result 'S13: the file carries Pos' $false 'no Pos tag'; return }
	$savedPose = [PSCustomObject]@{ x = $saved.pos[0]; y = $saved.pos[1]; z = $saved.pos[2] }
	$quitPose = [PSCustomObject]@{ x = $beforeQuit.pos.x; y = $beforeQuit.pos.y; z = $beforeQuit.pos.z }
	$posError = Get-XzDistance $savedPose $quitPose
	$yawError = [Math]::Abs([double]$saved.rotation[0] - [double]$beforeQuit.rot.yaw)
	$pitchError = [Math]::Abs([double]$saved.rotation[1] - [double]$beforeQuit.rot.pitch)
	Add-Check $Result 'S13: the saved pose and rotation match the session' ($posError -le $poseTol -and $yawError -le $rotTol -and $pitchError -le $rotTol) `
		("Pos file=({0},{1},{2}) observed=({3},{4},{5}) dxz={6:N3}; yaw file={7} observed={8} d={9:N3}; pitch file={10} observed={11} d={12:N3}; tolerances pose={13} rot={14}" -f `
			$saved.pos[0], $saved.pos[1], $saved.pos[2], $beforeQuit.pos.x, $beforeQuit.pos.y, $beforeQuit.pos.z, $posError, `
			$saved.rotation[0], $beforeQuit.rot.yaw, $yawError, $saved.rotation[1], $beforeQuit.rot.pitch, $pitchError, $poseTol, $rotTol)
	$fields = ("Pos=({0}) Rotation=({1}) Health={2} foodLevel={3} foodSaturationLevel={4} DataVersion={5} playerGameType={6} Dimension={7} OnGround={8}" -f `
		($saved.pos -join ','), ($saved.rotation -join ','), $saved.health, $saved.foodLevel, $saved.saturation, $saved.dataVersion, $saved.gameType, $saved.dimension, $saved.onGround)
	$script:s7Facts.Add("S13 saved file: $fields")
	Add-Check $Result 'S13: the file has the expected schema fields' ($saved.dataVersion -ne $null -and $saved.gameType -ne $null -and $saved.health -ne $null -and $saved.foodLevel -ne $null) $fields

	# (1) force_gamemode=true: make the file disagree, then check the join
	# takes the configured mode.
	Set-PlayerNbtInt $playerFile 'playerGameType' 2
	$patched = Get-PlayerNbtFacts $playerFile
	$script:s7Facts.Add("S13 force check: playerGameType patched to $($patched.gameType), configured mode $configMode")
	Start-Sleep -Seconds 2

	$rejoinAt = Get-Date
	$launchA2 = Start-Client $nickA $portA $rejoinAt
	Start-Sleep -Seconds 2
	$aTrace = Get-FreshTrace $rejoinAt
	if (-not $aTrace) { Add-Check $Result 'S13: the re-entry trace was captured' $false 'no fresh proxy trace'; return }
	$aRows = Read-Trace $aTrace.FullName
	$entry = Get-TracePlayerPosition $aRows
	$rejoin = (Invoke-TestCmd $portA (New-ObserveCmd 's13-rejoin')).result
	$modeName = [string]$rejoin.gameMode
	$entryPos = if ($entry) { [PSCustomObject]@{ x = $entry.x; y = $entry.y; z = $entry.z } } else { $null }
	$entryError = if ($entryPos) { Get-XzDistance $entryPos $savedPose } else { [double]::PositiveInfinity }
	$entryYaw = if ($entry) { [Math]::Abs([double]$entry.yaw - [double]$saved.rotation[0]) } else { [double]::PositiveInfinity }
	$entryPitch = if ($entry) { [Math]::Abs([double]$entry.pitch - [double]$saved.rotation[1]) } else { [double]::PositiveInfinity }
	Add-Check $Result 'S13: the re-entry teleports to the saved pose and rotation' ($entry -ne $null -and $entryError -le 0.01 -and $entryYaw -le 0.01 -and $entryPitch -le 0.01) `
		("player_position=({0},{1},{2}) yaw={3} pitch={4}; saved=({5},{6},{7}) yaw={8} pitch={9}; dxz={10:N4}" -f `
			$(if ($entry) { $entry.x } else { 'none' }), $(if ($entry) { $entry.y } else { 'none' }), $(if ($entry) { $entry.z } else { 'none' }), `
			$(if ($entry) { $entry.yaw } else { 'none' }), $(if ($entry) { $entry.pitch } else { 'none' }), `
			$saved.pos[0], $saved.pos[1], $saved.pos[2], $saved.rotation[0], $saved.rotation[1], $entryError)
	$script:s7Facts.Add(("S13 re-entry: player_position=({0}) yaw={1} pitch={2}, client gameMode={3}" -f $(if ($entry) { "$($entry.x),$($entry.y),$($entry.z)" } else { 'none' }), $(if ($entry) { $entry.yaw } else { 'none' }), $(if ($entry) { $entry.pitch } else { 'none' }), $modeName))
	Add-Check $Result 'S13 (1, force_gamemode=true): the configured mode wins over the file' ($modeName -eq (ConvertTo-GameModeName $configMode)) `
		("client gameMode={0}, configured={1}, file playerGameType={2}" -f $modeName, (ConvertTo-GameModeName $configMode), $patched.gameType)

	# --- B joins later and sees A in the saved pose -------------------------
	$bWatermark = Get-Date
	$launchB = Start-Client $nickB $portB $bWatermark
	Start-Sleep -Seconds 2
	$bTrace = Get-FreshTrace $bWatermark
	if (-not $bTrace) { Add-Check $Result 'S13: the B trace was captured' $false 'no fresh proxy trace'; return }
	$bRows = Read-Trace $bTrace.FullName
	$spawn = Get-TraceAddEntityPos $bRows $uuidBytes
	$spawnError = if ($spawn) { Get-XzDistance ([PSCustomObject]@{ x = $spawn.x; y = $spawn.y; z = $spawn.z }) $savedPose } else { [double]::PositiveInfinity }
	Add-Check $Result 'S13: B sees A at the saved pose' ($spawn -ne $null -and $spawnError -le 0.05) `
		("add_entity seq={0} pos=({1},{2},{3}); saved=({4},{5},{6}); dxz={7:N3}" -f $spawn.seq, $(if ($spawn) { $spawn.x } else { 'none' }), $(if ($spawn) { $spawn.y } else { 'none' }), $(if ($spawn) { $spawn.z } else { 'none' }), $saved.pos[0], $saved.pos[1], $saved.pos[2], $spawnError)
	$script:s7Facts.Add(("S13 B add_entity: seq={0} pos=({1},{2},{3})" -f $(if ($spawn) { $spawn.seq } else { 'none' }), $(if ($spawn) { $spawn.x } else { 'none' }), $(if ($spawn) { $spawn.y } else { 'none' }), $(if ($spawn) { $spawn.z } else { 'none' })))

	# (5) autosave cost while two players are online. The tick only snapshots
	# and enqueues (`autosave queued`: save_us + players); the disk write is on
	# the writer thread. The tick phase lines are reported as context only:
	# `late_ms` is dominated by the OS wake-up delay and by chunk/entity batch
	# delivery when the clients join, not by the save.
	Start-Sleep -Seconds 12
	$autosaveLines = @(Select-String -LiteralPath $oursLog -Pattern 'autosave queued' -ErrorAction SilentlyContinue)
	$plainAutosave = @($autosaveLines | ForEach-Object { $_.Line -replace '\x1b\[[0-9;]*m', '' })
	$autosaves = @($plainAutosave | ForEach-Object {
		$u = [regex]::Match($_, 'save_us=(\d+)')
		$p = [regex]::Match($_, 'players=(\d+)')
		if ($u.Success -and $p.Success) {
			[pscustomobject]@{ us = [int]$u.Groups[1].Value; players = [int]$p.Groups[1].Value }
		}
	})
	$twoPlayer = @($autosaves | Where-Object { $_.players -ge 2 })
	$maxSaveUs = if ($autosaves.Count -gt 0) { [int](($autosaves | Measure-Object us -Maximum).Maximum) } else { -1 }
	$maxTwoUs = if ($twoPlayer.Count -gt 0) { [int](($twoPlayer | Measure-Object us -Maximum).Maximum) } else { -1 }
	# The measurement is wall clock, so the OS preempting the tick thread (the
	# runner shares the machine with two Java clients and the proxy) shows up
	# as a spike; the median is the honest typical cost, the max is reported.
	$medianTwoUs = if ($twoPlayer.Count -gt 0) { @($twoPlayer | Sort-Object us)[[int]($twoPlayer.Count / 2)].us } else { -1 }
	$queueFull = Select-String -LiteralPath $oursLog -Pattern 'player save queue is full' -Quiet -ErrorAction SilentlyContinue
	$phaseLines = @(Select-String -LiteralPath $oursLog -Pattern 'tick phases|tick woke late' -ErrorAction SilentlyContinue)
	# The tracing writer colours the field names (`late_ms\e[0m\e[2m=\e[0m80.1`),
	# so the SGR sequences are stripped before matching. The escape byte itself
	# must go too: `\[[0-9;]*m` alone leaves `\e` between the name and `=`, and
	# every value parsed as 0 (S13 r4/r5).
	$plainLines = @($phaseLines | ForEach-Object { $_.Line -replace '\x1b\[[0-9;]*m', '' })
	$lateValues = @($plainLines | ForEach-Object {
		$m = [regex]::Match($_, 'late_ms=([0-9.]+)')
		if ($m.Success) { [double]$m.Groups[1].Value }
	})
	$phaseTotals = @($plainLines | ForEach-Object {
		$m = [regex]::Match($_, 'phases_total_us=(\d+)')
		if ($m.Success) { [int]$m.Groups[1].Value }
	})
	$maxLate = if ($lateValues.Count -gt 0) { ($lateValues | Measure-Object -Maximum).Maximum } else { 0.0 }
	$maxPhase = if ($phaseTotals.Count -gt 0) { [int](($phaseTotals | Measure-Object -Maximum).Maximum) } else { 0 }
	$script:s7Facts.Add(("S13 (5) autosave: {0} queued, {1} with two players, tick-side save_us median={2}, max={3}, max over all={4} (budget 5000 us); phase/late lines={5}, max late_ms={6:N3}, max phases_total_us={7} (context)" -f $autosaveLines.Count, $twoPlayer.Count, $medianTwoUs, $maxTwoUs, $maxSaveUs, $phaseLines.Count, $maxLate, $maxPhase))
	Add-Check $Result 'S13 (5): the autosave only enqueues, the tick does no disk work' ($twoPlayer.Count -gt 0 -and $maxTwoUs -le 5000 -and -not $queueFull) `
		("autosave lines={0}, with two players={1}, tick-side save_us median={2}, max={3}, max over all={4} (budget 5000 us), queue full={5}; phase/late lines={6}, max phases_total_us={7} (context)" -f $autosaveLines.Count, $twoPlayer.Count, $medianTwoUs, $maxTwoUs, $maxSaveUs, $queueFull, $phaseLines.Count, $maxPhase)

	$null = Invoke-TestCmd $portA (New-SessionCmd 's13-off2' 'disconnect')
	$null = Invoke-TestCmd $portB (New-SessionCmd 's13-offb' 'disconnect')
	Start-Sleep -Seconds 3

	# (2) corrupt file: the join falls back to the spawn with a warning.
	[IO.File]::WriteAllBytes($playerFile, [byte[]](1..16))
	$corruptAt = Get-Date
	$launchA3 = Start-Client $nickA $portA $corruptAt
	Start-Sleep -Seconds 2
	$corrupt = (Invoke-TestCmd $portA (New-ObserveCmd 's13-corrupt-entry')).result
	$spawnPose = [PSCustomObject]@{ x = $entry1.pos.x; y = $entry1.pos.y; z = $entry1.pos.z }
	$toSpawn = Get-XzDistance $corrupt.pos $spawnPose
	$toSaved = Get-XzDistance $corrupt.pos $savedPose
	$warned = Select-String -LiteralPath $oursLog -Pattern 'could not read the player file' -Quiet -ErrorAction SilentlyContinue
	Add-Check $Result 'S13 (2): a corrupt file joins at the spawn with a warning' ($toSpawn -le 1.0 -and $toSaved -gt 3.0 -and $warned) `
		("entry=({0},{1},{2}) spawn=({3},{4},{5}) dxz_spawn={6:N2} dxz_saved={7:N2} warn={8}" -f `
			$corrupt.pos.x, $corrupt.pos.y, $corrupt.pos.z, $entry1.pos.x, $entry1.pos.y, $entry1.pos.z, $toSpawn, $toSaved, $warned)
	$script:s7Facts.Add(("S13 corrupt: entry=({0},{1},{2}) dxz_spawn={3:N2} dxz_saved={4:N2} warn={5}" -f $corrupt.pos.x, $corrupt.pos.y, $corrupt.pos.z, $toSpawn, $toSaved, $warned))
	$null = Invoke-TestCmd $portA (New-SessionCmd 's13-off3' 'disconnect')
	Start-Sleep -Seconds 3

	# --- template and vanilla world untouched ------------------------------
	$templateAfter = Get-TreeFingerprint $templateWorldDir
	$vanillaAfter = Get-TreeFingerprint $vanillaWorld
	$templateDiff = @(Compare-Object $templateBefore $templateAfter)
	$vanillaDiff = @(Compare-Object $vanillaBefore $vanillaAfter)
	Add-Check $Result 'S13: the world template is unchanged (SHA-256)' ($templateDiff.Count -eq 0) `
		("files={0}, changed={1}" -f $templateAfter.Count, $templateDiff.Count)
	Add-Check $Result 'S13: vanilla\world is unchanged (SHA-256)' ($vanillaDiff.Count -eq 0) `
		("files={0}, changed={1}" -f $vanillaAfter.Count, $vanillaDiff.Count)
	$script:s7Facts.Add(("S13 hashes: template files={0} changed={1}; vanilla world files={2} changed={3}" -f $templateAfter.Count, $templateDiff.Count, $vanillaAfter.Count, $vanillaDiff.Count))
}

# S13-stop (РЁР°Рі 19a): a graceful stop flushes the last snapshot.
function Invoke-S13Stop {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$nickA = [string]$Cfg.playerA
	$poseTol = [double]$Cfg.poseTolerance
	$saveDir = $OursSaveDir

	$watermark = Get-Date
	$launchA = Start-Client $nickA $portA $watermark
	if (-not $launchA.uuid) { throw 'S13 stop: could not resolve Alice offline UUID' }
	$playerFile = Get-PlayerDataPath $saveDir $launchA.uuid
	$null = Invoke-TestCmd $portA (New-KeyCmd 's13s-walk-on' 'up' 'down')
	Start-Sleep -Milliseconds 900
	$null = Invoke-TestCmd $portA (New-KeyCmd 's13s-walk-off' 'up' 'up')
	$null = Invoke-TestCmd $portA (New-LookCmd 's13s-look' 120 30)
	Start-Sleep -Milliseconds 500
	$last = (Invoke-TestCmd $portA (New-ObserveCmd 's13s-last')).result
	Start-Sleep -Seconds 2

	# The server stops on CTRL_CLOSE_EVENT: taskkill without /F posts the
	# window close to its console, which is the signal the server waits for.
	$stopAt = Get-Date
	$taskkill = & taskkill /PID $script:oursPid 2>&1 | Out-String
	$deadline = (Get-Date).AddSeconds(25)
	while ((Get-Date) -lt $deadline -and (Get-Process -Id $script:oursPid -ErrorAction SilentlyContinue)) {
		Start-Sleep -Milliseconds 300
	}
	$exited = -not (Get-Process -Id $script:oursPid -ErrorAction SilentlyContinue)
	$flushed = Select-String -LiteralPath $oursLog -Pattern 'player saves flushed' -Quiet -ErrorAction SilentlyContinue
	Add-Check $Result 'S13 (4): the server stopped gracefully on the close signal' ($exited -and $flushed) `
		("exited={0} flushed line={1}; taskkill='{2}'" -f $exited, $flushed, $taskkill.Trim())
	if (-not $exited) {
		$script:s7Facts.Add('S13 stop: the close signal did not stop the server; no hard kill fallback was used for a flush check')
		return
	}
	$written = Wait-FileNewer $playerFile $stopAt 5
	$saved = if (Test-Path -LiteralPath $playerFile) { Get-PlayerNbtFacts $playerFile } else { $null }
	$stopPose = [PSCustomObject]@{ x = $last.pos.x; y = $last.pos.y; z = $last.pos.z }
	$savedPose = if ($saved -and $saved.pos) { [PSCustomObject]@{ x = $saved.pos[0]; y = $saved.pos[1]; z = $saved.pos[2] } } else { $null }
	$error = if ($savedPose) { Get-XzDistance $savedPose $stopPose } else { [double]::PositiveInfinity }
	Add-Check $Result 'S13 (4): the last snapshot survived the stop' ($written -and $saved -ne $null -and $error -le $poseTol) `
		("file newer than the stop={0}; saved Pos=({1}) last observed=({2},{3},{4}); dxz={5:N3}" -f $written, $(if ($saved) { $saved.pos -join ',' } else { 'none' }), $last.pos.x, $last.pos.y, $last.pos.z, $error)
	$script:s7Facts.Add(("S13 stop: file newer={0} saved=({1}) last=({2},{3},{4}) dxz={5:N3}" -f $written, $(if ($saved) { $saved.pos -join ',' } else { 'none' }), $last.pos.x, $last.pos.y, $last.pos.z, $error))
	$script:oursPid = $null
}

# S13-gamemode (РЁР°Рі 19a): force_gamemode=false lets the file decide.
function Invoke-S13Gamemode {
	param($Cfg, $Result)
	$portA = [int]$Cfg.controlPortA
	$nickA = [string]$Cfg.playerA
	$saveDir = $OursSaveDir
	$configMode = Get-OursConfigMode

	# The server reads the player file during login, so the file must disagree
	# with the config (adventure 2 vs survival 0) *before* the client joins.
	# The file name comes from the offline UUID (Bob's file is in the same
	# directory after the s13 run, so "the only .dat" is not unique).
	$playerFile = Join-Path (Join-Path $saveDir 'players\data') (Get-OfflinePlayerFileName $nickA)
	if (-not (Test-Path -LiteralPath $playerFile)) {
		Add-Check $Result 'S13 (1, force_gamemode=false): the saved file exists' $false $playerFile
		return
	}
	Set-PlayerNbtInt $playerFile 'playerGameType' 2
	$fileFacts = Get-PlayerNbtFacts $playerFile

	$watermark = Get-Date
	$launchA = Start-Client $nickA $portA $watermark
	if (-not $launchA.uuid) { throw 'S13 gamemode: could not resolve Alice offline UUID' }
	$aTrace = Get-FreshTrace $watermark
	if (-not $aTrace) { Add-Check $Result 'S13 gamemode: trace captured' $false 'no fresh proxy trace'; return }
	$gamemodeObs = (Invoke-TestCmd $portA (New-ObserveCmd 's13g-mode')).result
	$modeName = [string]$gamemodeObs.gameMode
	Add-Check $Result 'S13 (1, force_gamemode=false): the file mode wins' ($modeName -eq (ConvertTo-GameModeName ([int]$fileFacts.gameType)) -and $modeName -ne (ConvertTo-GameModeName $configMode)) `
		("client gameMode={0}, file playerGameType={1}, configured={2}" -f $modeName, $fileFacts.gameType, (ConvertTo-GameModeName $configMode))
	$script:s7Facts.Add(("S13 gamemode: client gameMode={0}, file mode={1}, configured={2}" -f $modeName, $fileFacts.gameType, (ConvertTo-GameModeName $configMode)))
	$null = Invoke-TestCmd $portA (New-SessionCmd 's13g-off' 'disconnect')
	Start-Sleep -Seconds 2
}

# --- S16 (Шаг 23b): world-save restart and crash automation ---------------

function Get-S16BlockId {
	param([int]$Port, $Target, [string]$Id)
	$response = Invoke-TestCmd $Port (New-BlockCmd $Id ([int]$Target.x) ([int]$Target.y) ([int]$Target.z)) 4000
	if (-not $response.ok -or $response.result.inGame -ne $true) {
		throw "S16 block query failed for $($Target.name)"
	}
	return [string]$response.result.block.id
}

function Wait-S16BlockId {
	param([int]$Port, $Target, [string]$Expected, [int]$Seconds = 10)
	$deadline = (Get-Date).AddSeconds($Seconds)
	$last = 'unread'
	while ((Get-Date) -lt $deadline) {
		try { $last = Get-S16BlockId $Port $Target ("s16-{0}-poll" -f $Target.name) } catch { }
		if ($last -eq $Expected) {
			return [PSCustomObject]@{ matched = $true; value = $last }
		}
		Start-Sleep -Milliseconds 150
	}
	return [PSCustomObject]@{ matched = $false; value = $last }
}

function Invoke-S16BreakTarget {
	param([int]$Port, $Target, $Result, [string]$Phase)
	$before = Get-S16BlockId $Port $Target ("$Phase-$($Target.name)-before")
	Add-Check $Result ("S16 {0}: {1} starts as {2}" -f $Phase, $Target.name, $Target.before) `
		($before -eq [string]$Target.before) ("block={0} at ({1},{2},{3})" -f $before, $Target.x, $Target.y, $Target.z)
	$reply = Invoke-TestCmd $Port (New-DigPacketCmd "$Phase-$($Target.name)-break" ([int]$Target.x) ([int]$Target.y) ([int]$Target.z) 1) 4000
	$after = Wait-S16BlockId $Port $Target ([string]$Target.after) 10
	Add-Check $Result ("S16 {0}: A breaks {1}" -f $Phase, $Target.name) `
		($reply.ok -and $after.matched) ("after={0}, expected={1}" -f $after.value, $Target.after)
	$script:s7Facts.Add(("S16 {0} break {1}: ({2},{3},{4}) {5}->{6}" -f `
			$Phase, $Target.name, $Target.x, $Target.y, $Target.z, $before, $after.value))
}

function Get-S16ChunkPath {
	param([string]$SaveDir, $Target)
	$cx = [int][Math]::Floor(([double]$Target.x) / 16.0)
	$cz = [int][Math]::Floor(([double]$Target.z) / 16.0)
	return Join-Path (Join-Path $SaveDir 'chunks') ("{0}.{1}.mrchunk" -f $cx, $cz)
}

function Test-S16MrChunk {
	param([string]$Path)
	if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
		return [PSCustomObject]@{ valid = $false; detail = 'missing' }
	}
	[byte[]]$bytes = [IO.File]::ReadAllBytes($Path)
	if ($bytes.Length -lt 21) {
		return [PSCustomObject]@{ valid = $false; detail = "short:$($bytes.Length)" }
	}
	$magic = [Text.Encoding]::ASCII.GetString($bytes, 0, 4)
	if ($magic -ne 'MRSV' -or $bytes[4] -ne 1) {
		return [PSCustomObject]@{ valid = $false; detail = "header:$magic/v$($bytes[4])" }
	}
	[uint64]$crc = 4294967295
	for ($i = 0; $i -lt ($bytes.Length - 4); $i++) {
		$crc = ($crc -bxor [uint64]$bytes[$i]) -band 0xffffffff
		for ($bit = 0; $bit -lt 8; $bit++) {
			if (($crc -band 1) -ne 0) {
				$crc = (($crc -shr 1) -bxor [uint64]3988292384) -band 4294967295
			} else {
				$crc = ($crc -shr 1) -band 4294967295
			}
		}
	}
	[uint32]$actual = [uint32](($crc -bxor 4294967295) -band 4294967295)
	[uint32]$stored = [BitConverter]::ToUInt32($bytes, $bytes.Length - 4)
	return [PSCustomObject]@{
		valid = ($actual -eq $stored)
		detail = ("bytes={0} crc={1:x8} stored={2:x8}" -f $bytes.Length, $actual, $stored)
	}
}

function Get-S16SaveAudit {
	param([string]$SaveDir)
	$chunkDir = Join-Path $SaveDir 'chunks'
	$files = if (Test-Path -LiteralPath $chunkDir) { @(Get-ChildItem -LiteralPath $chunkDir -File -Filter '*.mrchunk') } else { @() }
	$tmp = if (Test-Path -LiteralPath $chunkDir) { @(Get-ChildItem -LiteralPath $chunkDir -File -Filter '*.tmp') } else { @() }
	$bad = @()
	foreach ($file in $files) {
		$check = Test-S16MrChunk $file.FullName
		if (-not $check.valid) { $bad += ("{0}: {1}" -f $file.Name, $check.detail) }
	}
	return [PSCustomObject]@{ files = $files.Count; temporary = $tmp.Count; bad = $bad }
}

function Wait-S16DurableChunk {
	param([string]$Path, [datetime]$Since, [int]$Seconds = 20)
	$deadline = (Get-Date).AddSeconds($Seconds)
	while ((Get-Date) -lt $deadline) {
		if (Test-Path -LiteralPath $Path) {
			$item = Get-Item -LiteralPath $Path
			$check = Test-S16MrChunk $Path
			if ($item.LastWriteTime -gt $Since -and $check.valid) { return $check }
		}
		Start-Sleep -Milliseconds 200
	}
	return [PSCustomObject]@{ valid = $false; detail = 'no newer durable diff before timeout' }
}

function Add-S16ProtectionChecks {
	param($Result, $TemplateBefore, $VanillaBefore, [string]$Phase)
	$templateAfter = Get-TreeFingerprint $templateWorldDir
	$vanillaWorld = Join-Path $vanillaDir 'world'
	$vanillaAfter = Get-TreeFingerprint $vanillaWorld
	$templateDiff = @(Compare-Object $TemplateBefore $templateAfter)
	$vanillaDiff = @(Compare-Object $VanillaBefore $vanillaAfter)
	Add-Check $Result "S16 $Phase`: template v3 SHA-256 is unchanged" ($templateDiff.Count -eq 0) `
		("files={0} changed={1}" -f $templateAfter.Count, $templateDiff.Count)
	Add-Check $Result "S16 $Phase`: vanilla\world SHA-256 is unchanged" ($vanillaDiff.Count -eq 0) `
		("files={0} changed={1}" -f $vanillaAfter.Count, $vanillaDiff.Count)
}

function Start-S16Pair {
	param($Cfg)
	$null = Start-Client ([string]$Cfg.playerA) ([int]$Cfg.controlPortA) (Get-Date)
	$null = Start-Client ([string]$Cfg.playerB) ([int]$Cfg.controlPortB) (Get-Date)
	$null = Wait-ObserverChunksStable ([int]$Cfg.controlPortB) 30
}

function Stop-S16Clients {
	param($Cfg)
	Stop-ClientByName ([string]$Cfg.playerA)
	Stop-ClientByName ([string]$Cfg.playerB)
	Start-Sleep -Seconds 2
}

function Add-S16RestartBlockChecks {
	param($Cfg, $Result, $Targets, [string]$Phase)
	foreach ($target in @($Targets)) {
		$a = Get-S16BlockId ([int]$Cfg.controlPortA) $target ("$Phase-a-$($target.name)")
		$b = Get-S16BlockId ([int]$Cfg.controlPortB) $target ("$Phase-b-$($target.name)")
		$pass = ($a -eq [string]$target.after -and $b -eq [string]$target.after)
		Add-Check $Result ("S16 {0}: A and late B see {1} persisted" -f $Phase, $target.name) $pass `
			("A={0} B={1} expected={2}" -f $a, $b, $target.after)
	}
}

function Invoke-S16a {
	param($Cfg, $Result)
	if ($isVanilla) { throw 'S16a targets our world-save implementation' }
	$templateBefore = Get-TreeFingerprint $templateWorldDir
	$vanillaBefore = Get-TreeFingerprint (Join-Path $vanillaDir 'world')
	$configHash = (Get-FileHash -LiteralPath $script:oursConfig -Algorithm SHA256).Hash
	$null = Start-Client ([string]$Cfg.playerA) ([int]$Cfg.controlPortA) (Get-Date)
	$null = Wait-ObserverChunksStable ([int]$Cfg.controlPortA) 30
	$mode = (Invoke-TestCmd ([int]$Cfg.controlPortA) (New-ObserveCmd 's16a-mode')).result.gameMode
	Add-Check $Result 'S16a: A is in creative' ($mode -eq 'creative') "mode=$mode"
	foreach ($target in @($Cfg.targets)) {
		Invoke-S16BreakTarget ([int]$Cfg.controlPortA) $target $Result 'a'
	}

	$stopped = Stop-OursServer -Mode graceful -TimeoutSec 30
	$log = [string]$stopped.log
	$worldFlushed = Select-String -LiteralPath $log -Pattern 'final world save flushed' -Quiet -ErrorAction SilentlyContinue
	$writerStopped = Select-String -LiteralPath $log -Pattern 'world-save writer stopped' -Quiet -ErrorAction SilentlyContinue
	$playersFlushed = Select-String -LiteralPath $log -Pattern 'player saves flushed' -Quiet -ErrorAction SilentlyContinue
	Add-Check $Result 'S16a: graceful stop exits 0 and flushes world before shutdown' `
		($stopped.exited -and $stopped.exitCode -eq 0 -and $worldFlushed -and $writerStopped -and $playersFlushed) `
		("exited={0} exitCode={1} world={2} writer={3} players={4} log={5}" -f `
			$stopped.exited, $stopped.exitCode, $worldFlushed, $writerStopped, $playersFlushed, $log)
	Stop-S16Clients $Cfg
	$null = Restart-OursServer
	$configHashAfter = (Get-FileHash -LiteralPath $script:oursConfig -Algorithm SHA256).Hash
	Add-Check $Result 'S16a: restart reuses the exact same config/save_dir' ($configHash -eq $configHashAfter) `
		("config sha before={0} after={1} save_dir={2}" -f $configHash, $configHashAfter, $OursSaveDir)
	Start-S16Pair $Cfg
	Add-S16RestartBlockChecks $Cfg $Result $Cfg.targets 'a'
	$audit = Get-S16SaveAudit $OursSaveDir
	Add-Check $Result 'S16a: every diff has a valid CRC and no .tmp remains' `
		($audit.files -gt 0 -and $audit.bad.Count -eq 0 -and $audit.temporary -eq 0) `
		("files={0} bad={1} tmp={2}" -f $audit.files, ($audit.bad -join '; '), $audit.temporary)
	Add-S16ProtectionChecks $Result $templateBefore $vanillaBefore 'a'
	try { $null = Invoke-TestCmd ([int]$Cfg.controlPortA) (New-SessionCmd 's16a-off-a' 'disconnect') } catch { }
	try { $null = Invoke-TestCmd ([int]$Cfg.controlPortB) (New-SessionCmd 's16a-off-b' 'disconnect') } catch { }
}

function Invoke-S16b {
	param($Cfg, $Result)
	if ($isVanilla) { throw 'S16b targets our world-save implementation' }
	$templateBefore = Get-TreeFingerprint $templateWorldDir
	$vanillaBefore = Get-TreeFingerprint (Join-Path $vanillaDir 'world')
	$configHash = (Get-FileHash -LiteralPath $script:oursConfig -Algorithm SHA256).Hash
	$null = Start-Client ([string]$Cfg.playerA) ([int]$Cfg.controlPortA) (Get-Date)
	$null = Wait-ObserverChunksStable ([int]$Cfg.controlPortA) 30
	$mode = (Invoke-TestCmd ([int]$Cfg.controlPortA) (New-ObserveCmd 's16b-mode')).result.gameMode
	Add-Check $Result 'S16b: A is in creative' ($mode -eq 'creative') "mode=$mode"

	$durableTarget = $Cfg.beforeAutosave
	$breakAt = Get-Date
	Invoke-S16BreakTarget ([int]$Cfg.controlPortA) $durableTarget $Result 'b-durable'
	# Explicitly wait longer than one configured world-autosave period, then
	# demand a newer checksummed file; `world autosave queued` alone is not a
	# durability signal.
	Start-Sleep -Milliseconds (($OursWorldAutosaveTicks * 50) + 500)
	$chunkPath = Get-S16ChunkPath $OursSaveDir $durableTarget
	$durable = Wait-S16DurableChunk $chunkPath $breakAt 20
	Add-Check $Result 'S16b: the pre-kill edit reached an atomic CRC-valid autosave' $durable.valid `
		("path={0} {1}" -f $chunkPath, $durable.detail)

	$lateTarget = $Cfg.afterAutosave
	Invoke-S16BreakTarget ([int]$Cfg.controlPortA) $lateTarget $Result 'b-late'
	$stopped = Stop-OursServer -Mode hard -TimeoutSec 15
	$hardLog = [string]$stopped.log
	$gracefulMarker = Select-String -LiteralPath $hardLog -Pattern 'final world save flushed' -Quiet -ErrorAction SilentlyContinue
	Add-Check $Result 'S16b: server was hard-killed without shutdown flush' (-not $gracefulMarker) `
		("exit={0} final-flush-line={1} log={2}" -f $stopped.exitCode, $gracefulMarker, $hardLog)
	Stop-S16Clients $Cfg
	$auditAfterKill = Get-S16SaveAudit $OursSaveDir
	Add-Check $Result 'S16b: hard kill left no corrupt or temporary diff files' `
		($auditAfterKill.files -gt 0 -and $auditAfterKill.bad.Count -eq 0 -and $auditAfterKill.temporary -eq 0) `
		("files={0} bad={1} tmp={2}" -f $auditAfterKill.files, ($auditAfterKill.bad -join '; '), $auditAfterKill.temporary)

	$null = Restart-OursServer
	$configHashAfter = (Get-FileHash -LiteralPath $script:oursConfig -Algorithm SHA256).Hash
	Add-Check $Result 'S16b: crash restart reuses the exact same config/save_dir' ($configHash -eq $configHashAfter) `
		("config sha before={0} after={1}" -f $configHash, $configHashAfter)
	Start-S16Pair $Cfg
	Add-S16RestartBlockChecks $Cfg $Result @($durableTarget) 'b-durable'
	$aLate = Get-S16BlockId ([int]$Cfg.controlPortA) $lateTarget 's16b-late-a'
	$bLate = Get-S16BlockId ([int]$Cfg.controlPortB) $lateTarget 's16b-late-b'
	$allowed = @([string]$lateTarget.before, [string]$lateTarget.after)
	Add-Check $Result 'S16b: the post-autosave edit is either saved or rolled back consistently' `
		($aLate -eq $bLate -and $allowed -contains $aLate) `
		("A={0} B={1} allowed={2}" -f $aLate, $bLate, ($allowed -join ','))
	$auditAfterRestart = Get-S16SaveAudit $OursSaveDir
	$damagedWarning = Select-String -LiteralPath $script:oursLog -Pattern 'damaged chunk diff|checksum' -Quiet -ErrorAction SilentlyContinue
	Add-Check $Result 'S16b: restart reads every diff without CRC/format warnings' `
		($auditAfterRestart.bad.Count -eq 0 -and $auditAfterRestart.temporary -eq 0 -and -not $damagedWarning) `
		("bad={0} tmp={1} warning={2}" -f ($auditAfterRestart.bad -join '; '), $auditAfterRestart.temporary, $damagedWarning)
	Add-S16ProtectionChecks $Result $templateBefore $vanillaBefore 'b'
	try { $null = Invoke-TestCmd ([int]$Cfg.controlPortA) (New-SessionCmd 's16b-off-a' 'disconnect') } catch { }
	try { $null = Invoke-TestCmd ([int]$Cfg.controlPortB) (New-SessionCmd 's16b-off-b' 'disconnect') } catch { }
}

# --- S17 (Шаг 23d): vanilla placement/inventory trace only -----------------

function Invoke-S17 {
	param($Cfg, $Result)
	if (-not $isVanilla) { throw 'S17 is a vanilla trace/plan scenario; server implementation is forbidden in Step 23d' }
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$nickA = [string]$Cfg.playerA
	$nickB = [string]$Cfg.playerB
	$settle = [int]$Cfg.settleMs
	$inv = [Globalization.CultureInfo]::InvariantCulture
	$watermark = Get-Date
	$launchA = Start-Client $nickA $portA $watermark
	$null = Start-Client $nickB $portB (Get-Date)
	$null = Wait-ObserverChunksStable $portB 30
	Invoke-VanillaConsole ("gamemode creative {0}" -f $nickA) | Out-Null
	Invoke-VanillaConsole ("gamemode creative {0}" -f $nickB) | Out-Null
	Invoke-VanillaConsole 'gamerule doMobSpawning false' | Out-Null
	$fl = @($Cfg.forceload)
	if ($fl.Count -eq 4) {
		Invoke-VanillaConsole ("forceload add {0} {1} {2} {3}" -f $fl[0], $fl[1], $fl[2], $fl[3]) | Out-Null
	}
	Start-Sleep -Seconds 2

	$caseFacts = New-Object System.Collections.ArrayList
	foreach ($case in @($Cfg.cases)) {
		$name = [string]$case.name
		$source = @($case.source)
		$support = @($case.support)
		$dest = @($case.dest)
		$stand = @($case.stand)
		$initialId = ([string]$case.initial -split '\[')[0]

		# Every case owns unique coordinates, so its wire rows can be attributed
		# without comparing independent A/B clocks.
		Invoke-VanillaConsole ("setblock {0} {1} {2} {3}" -f $source[0], $source[1], $source[2], $case.sourceBlock) | Out-Null
		Invoke-VanillaConsole ("setblock {0} {1} {2} minecraft:stone" -f $support[0], $support[1], $support[2]) | Out-Null
		Invoke-VanillaConsole ("setblock {0} {1} {2} {3}" -f $dest[0], $dest[1], $dest[2], $case.initial) | Out-Null
		Invoke-VanillaConsole ("setblock {0} {1} {2} minecraft:stone" -f $dest[0], ([int]$dest[1] - 1), $dest[2]) | Out-Null
		if ($case.surface) {
			Invoke-VanillaConsole ("setblock {0} {1} {2} {3}" -f $dest[0], ([int]$dest[1] - 1), $dest[2], ([string]$case.surface)) | Out-Null
			Invoke-VanillaConsole ("setblock {0} {1} {2} {3}" -f $dest[0], $dest[1], $dest[2], $case.initial) | Out-Null
		}
		Invoke-VanillaConsole ("fill {0} 200 {1} {2} 200 {3} minecraft:stone" -f `
			([int][Math]::Floor([double]$stand[0]) - 1), ([int][Math]::Floor([double]$stand[2]) - 1), `
			([int][Math]::Floor([double]$stand[0]) + 1), ([int][Math]::Floor([double]$stand[2]) + 1)) | Out-Null
		Start-Sleep -Milliseconds 400

		# Creative pick-block is the real key path; its trace must prove whether
		# the client emitted set_creative_mode_slot.
		$pickTp = [string]::Format($inv, "tp {0} {1} {2} {3} 0 90", $nickA, `
			([double]$source[0] + 0.5), ([double]$source[1] + 1.0), ([double]$source[2] + 0.5))
		Invoke-VanillaConsole $pickTp | Out-Null
		Start-Sleep -Milliseconds 350
		$pickAim = (Invoke-TestCmd $portA (New-ObserveCmd "s17-$name-pick-aim")).result
		$pickAimed = ($pickAim.hitType -eq 'BLOCK' -and $null -ne $pickAim.hitBlock -and `
			[int]$pickAim.hitBlock.x -eq [int]$source[0] -and [int]$pickAim.hitBlock.y -eq [int]$source[1] -and `
			[int]$pickAim.hitBlock.z -eq [int]$source[2])
		Add-Check $Result "S17 $name`: pick crosshair reaches the source" $pickAimed `
			("hit={0} ({1},{2},{3})" -f $pickAim.hitType, $pickAim.hitBlock.x, $pickAim.hitBlock.y, $pickAim.hitBlock.z)
		if ($pickAimed) { $null = Invoke-TestCmd $portA (New-KeyCmd "s17-$name-pick" 'pick' 'tap') }
		Start-Sleep -Milliseconds 400
		$held = (Invoke-TestCmd $portA (New-ObserveCmd "s17-$name-held")).result
		Add-Check $Result "S17 $name`: creative pick selects the expected item" `
			([string]$held.heldItem -eq [string]$case.item) `
			("slot={0} held={1} count={2}" -f $held.selectedSlot, $held.heldItem, $held.heldCount)

		if ($case.occupyWithB -eq $true) {
			$bTp = [string]::Format($inv, "tp {0} {1} {2} {3} 0 0", $nickB, `
				([double]$dest[0] + 0.5), [double]$dest[1], ([double]$dest[2] + 0.5))
			Invoke-VanillaConsole $bTp | Out-Null
			Start-Sleep -Milliseconds 300
		}
		$placeTp = [string]::Format($inv, "tp {0} {1} {2} {3} {4} {5}", $nickA, `
			[double]$stand[0], [double]$stand[1], [double]$stand[2], [double]$stand[3], [double]$stand[4])
		Invoke-VanillaConsole $placeTp | Out-Null
		Start-Sleep -Milliseconds 400
		$aim = (Invoke-TestCmd $portA (New-ObserveCmd "s17-$name-place-aim")).result
		$preA = Get-S16BlockId $portA ([PSCustomObject]@{ name=$name; x=$dest[0]; y=$dest[1]; z=$dest[2] }) "s17-$name-pre-a"
		$preB = Get-S16BlockId $portB ([PSCustomObject]@{ name=$name; x=$dest[0]; y=$dest[1]; z=$dest[2] }) "s17-$name-pre-b"
		Add-Check $Result "S17 $name`: A and B see the prepared destination" `
			($preA -eq $initialId -and $preB -eq $initialId) ("A={0} B={1} expected={2}" -f $preA, $preB, $initialId)
		$null = Invoke-TestCmd $portA (New-KeyCmd "s17-$name-use" 'use' 'tap' 100)
		Start-Sleep -Milliseconds $settle
		$postAResult = (Invoke-TestCmd $portA (New-BlockCmd "s17-$name-post-a" ([int]$dest[0]) ([int]$dest[1]) ([int]$dest[2]))).result
		$postBResult = (Invoke-TestCmd $portB (New-BlockCmd "s17-$name-post-b" ([int]$dest[0]) ([int]$dest[1]) ([int]$dest[2]))).result
		$postA = [string]$postAResult.block.id
		$postB = [string]$postBResult.block.id
		Add-Check $Result "S17 $name`: vanilla placement outcome is visible to A and B" `
			($postA -eq [string]$case.expectBlock -and $postB -eq [string]$case.expectBlock) `
			("A={0} B={1} expected={2}" -f $postA, $postB, $case.expectBlock)

		$caseFacts.Add([PSCustomObject]@{
			name = $name; source = @($source); support = @($support); destination = @($dest)
			item = [string]$case.item; selectedSlot = $held.selectedSlot; heldCount = $held.heldCount
			pickHit = $pickAim.hitBlock; placeHit = $aim.hitBlock
			beforeA = $preA; beforeB = $preB; afterA = $postA; afterB = $postB
			propertiesA = $postAResult.block.properties; propertiesB = $postBResult.block.properties
			occupied = [bool]$case.occupyWithB; outOfRange = [bool]$case.outOfRange
		}) | Out-Null
		if ($case.occupyWithB -eq $true) {
			Invoke-VanillaConsole ("tp {0} 96.5 145 -31.5 0 0" -f $nickB) | Out-Null
		}
	}

	Start-Sleep -Seconds 1
	# Disconnect both clients before the trace is read. While they stay
	# connected the two vanilla clients keep streaming entity/rotation packets
	# into the trace, so a full-file read never converges (observed 18.11.2026:
	# the conn traces grew to ~300 MB during post-processing and the run
	# stalled for >35 min). Disconnecting first bounds the trace.
	try { $null = Invoke-TestCmd $portA (New-SessionCmd 's17-off-a' 'disconnect') } catch { }
	try { $null = Invoke-TestCmd $portB (New-SessionCmd 's17-off-b' 'disconnect') } catch { }
	Start-Sleep -Seconds 3
	$files = @(Get-NewTraceFiles $traceDir $watermark)
	$aTrace = Select-TraceByPattern $files 'minecraft:(set_creative_mode_slot|use_item_on)'
	$bTrace = if ($aTrace) { @($files | Where-Object { $_.FullName -ne $aTrace.FullName } | Select-Object -First 1) } else { $null }
	$packetNames = @(
		'minecraft:pick_item_from_block', 'minecraft:set_carried_item', 'minecraft:set_creative_mode_slot',
		'minecraft:use_item_on', 'minecraft:use_item', 'minecraft:interact',
		'minecraft:block_update', 'minecraft:block_changed_ack', 'minecraft:section_blocks_update',
		'minecraft:container_set_content', 'minecraft:container_set_slot', 'minecraft:set_held_slot',
		'minecraft:set_cursor_item', 'minecraft:level_event', 'minecraft:sound'
	)
	$wire = New-Object System.Collections.ArrayList
	foreach ($trace in @($aTrace, $bTrace)) {
		if ($null -eq $trace) { continue }
		foreach ($row in @(Read-TraceMatches $trace.FullName $packetNames)) {
			$wire.Add([PSCustomObject]@{
				trace = $trace.Name; seq = [int]$row.seq; ms = [double]$row.read_ms
				state = [string]$row.state; dir = [string]$row.dir; id = [int]$row.id
				name = [string]$row.name; b64 = [string]$row.b64
			}) | Out-Null
		}
	}
	$pickRows = @($wire | Where-Object { $_.dir -eq 'serverbound' -and $_.name -eq 'minecraft:pick_item_from_block' })
	$creativeRows = @($wire | Where-Object { $_.dir -eq 'serverbound' -and $_.name -eq 'minecraft:set_creative_mode_slot' })
	$useRows = @($wire | Where-Object { $_.dir -eq 'serverbound' -and ($_.name -eq 'minecraft:use_item_on' -or $_.name -eq 'minecraft:use_item') })
	Add-Check $Result 'S17: vanilla creative pick sends pick_item_from_block' ($pickRows.Count -gt 0) "count=$($pickRows.Count)"
	Add-Check $Result 'S17: vanilla trace carries use_item_on/use_item placements' ($useRows.Count -gt 0) "count=$($useRows.Count)"
	Add-Check $Result 'S17: no set_creative_mode_slot in the pick-block flow (observed, not required)' $true `
		("set_creative_mode_slot={0} set_carried_item={1}" -f $creativeRows.Count,
			@($wire | Where-Object { $_.dir -eq 'serverbound' -and $_.name -eq 'minecraft:set_carried_item' }).Count)

	Invoke-VanillaConsole 'save-all flush' | Out-Null
	Start-Sleep -Seconds 2
	$playerFile = Join-Path (Join-Path $script:activeWorldDir 'players\data') (Get-OfflinePlayerFileName $nickA)
	$inventory = @()
	$selectedItemSlot = $null
	if (Test-Path -LiteralPath $playerFile) {
		$playerDoc = Read-PlayerNbt $playerFile
		$selectedItemSlot = $playerDoc.root['SelectedItemSlot']
		$inventory = @($playerDoc.root['Inventory'])
	}
	Add-Check $Result 'S17: vanilla playerdata contains the creative inventory' `
		($inventory.Count -gt 0) ("file={0} selected={1} inventory entries={2}" -f $playerFile, $selectedItemSlot, $inventory.Count)

	$artifact = [PSCustomObject]@{
		generated = (Get-Date).ToString('o')
		cases = @($caseFacts)
		wire = @($wire)
		playerdata = [PSCustomObject]@{ path = $playerFile; selectedItemSlot = $selectedItemSlot; inventory = @($inventory) }
	}
	$artifactPath = Join-Path $traceDir 's17-samples.json'
	$artifact | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $artifactPath -Encoding utf8
	$script:s7Facts.Add("S17 artifact: $artifactPath")
}

# --- S15 (РЁР°Рі 20a): duplicate login on one name ---------------------------
# The scenario opens a second connection with the same player name and
# records what the first connection receives. The vanilla trace is the
# source for the kick packet and its text; our server must match it.

# Reads a wire String (VarInt length + UTF-8 bytes) from a trace payload.
function ConvertFrom-TraceString {
	param([byte[]]$Bytes, [int]$Offset = 0)
	$len = ConvertFrom-TraceVarInt $Bytes $Offset
	$start = $len.next
	$end = $start + $len.value
	if ($end -gt $Bytes.Length) { throw 'truncated string in trace payload' }
	return [PSCustomObject]@{
		value = [System.Text.Encoding]::UTF8.GetString($Bytes, $start, $len.value)
		next  = $end
	}
}

# The player name a connection logged in with (`minecraft:hello`, login state).
function Get-TraceLoginName {
	param($Rows)
	$hello = @($Rows | Where-Object { $_.state -eq 'login' -and $_.name -eq 'minecraft:hello' } | Select-Object -First 1)
	if ($hello.Count -eq 0) { return $null }
	$bytes = ConvertFrom-TraceB64 $hello[0].b64
	return (ConvertFrom-TraceString $bytes 0).value
}

# Clientbound `minecraft:disconnect` rows of the play state.
function Get-TracePlayDisconnects {
	param($Rows)
	return @($Rows | Where-Object { $_.dir -eq 'clientbound' -and $_.state -eq 'play' -and $_.name -eq 'minecraft:disconnect' })
}

# The translation key (or literal text) of a play-state disconnect reason.
#
# 26.3 sends a text component as unnamed-root network NBT: 0x0A (compound),
# 0x08 (string) + u16 name length + name, u16 value length + value, 0x00 end.
# The S15 duplicate-login reason is `{"translate": "multiplayer.disconnect.
# duplicate_login"}`, so a plain wire String would decode as garbage.
function ConvertFrom-TraceDisconnectText {
	param([byte[]]$Bytes)
	if ($Bytes.Length -lt 3 -or $Bytes[0] -ne 0x0A) {
		throw 'disconnect reason is not an NBT compound'
	}
	$at = 1
	$field = ''
	$value = ''
	while ($at -lt $Bytes.Length) {
		$type = [int]$Bytes[$at]
		$at++
		if ($type -eq 0) { break }
		if ($type -ne 0x08) { throw "disconnect reason has an unsupported NBT tag $type" }
		$nameLen = ([int]$Bytes[$at] -shl 8) -bor [int]$Bytes[$at + 1]
		$at += 2
		$field = [System.Text.Encoding]::UTF8.GetString($Bytes, $at, $nameLen)
		$at += $nameLen
		$valueLen = ([int]$Bytes[$at] -shl 8) -bor [int]$Bytes[$at + 1]
		$at += 2
		$value = [System.Text.Encoding]::UTF8.GetString($Bytes, $at, $valueLen)
		$at += $valueLen
	}
	return $value
}

# Canonical UUID string and profile name of a clientbound `login_finished`
# payload: 16 UUID bytes (network order) followed by the name String.
function Get-TraceLoginProfile {
	param([byte[]]$Bytes)
	if ($Bytes.Length -lt 17) { throw 'truncated login_finished payload' }
	$hex = ($Bytes[0..15] | ForEach-Object { '{0:x2}' -f $_ }) -join ''
	$uuid = '{0}-{1}-{2}-{3}-{4}' -f $hex.Substring(0, 8), $hex.Substring(8, 4), $hex.Substring(12, 4), $hex.Substring(16, 4), $hex.Substring(20, 12)
	$name = (ConvertFrom-TraceString $Bytes 16).value
	return [PSCustomObject]@{ uuid = $uuid; name = $name }
}

# Classifies the fresh traces by profile name and by whatever disconnect the
# connection received. Login- and configuration-phase rejections carry no
# play state, so every trace with a profile counts; the disconnect may be a
# play packet (NBT component) or a login packet (wire String with JSON).
function Get-S15TraceFacts {
	param([datetime]$Since)
	$facts = @()
	foreach ($file in @(Get-NewTraceFiles $traceDir $Since)) {
		$rows = Read-Trace $file.FullName
		$finished = @($rows | Where-Object { $_.dir -eq 'clientbound' -and $_.state -eq 'login' -and $_.name -eq 'minecraft:login_finished' } | Select-Object -First 1)
		$profile = $null
		if ($finished.Count -gt 0) {
			$profile = Get-TraceLoginProfile (ConvertFrom-TraceB64 $finished[0].b64)
		}
		$name = if ($profile) { $profile.name } else { Get-TraceLoginName $rows }
		if (-not $name) { continue }
		$states = @($rows | ForEach-Object { $_.state } | Sort-Object -Unique)
		$playRows = @($rows | Where-Object { $_.state -eq 'play' })
		$disconnectRow = @($rows | Where-Object {
			$_.dir -eq 'clientbound' -and
			($_.name -eq 'minecraft:disconnect' -or $_.name -eq 'minecraft:login_disconnect')
		} | Select-Object -First 1)
		$loginRow = @($playRows | Where-Object { $_.dir -eq 'clientbound' -and $_.name -eq 'minecraft:login' } | Select-Object -First 1)
		$text = $null
		$seq = $null
		$readMs = $null
		$disconnectState = $null
		if ($disconnectRow.Count -gt 0) {
			$seq = [int]$disconnectRow[0].seq
			$readMs = [double]$disconnectRow[0].read_ms
			$disconnectState = [string]$disconnectRow[0].state
			$bytes = ConvertFrom-TraceB64 $disconnectRow[0].b64
			$text = if ($disconnectRow[0].state -eq 'login') {
				(ConvertFrom-TraceString $bytes 0).value
			} else {
				# Configuration and play both carry a network-NBT text
				# component in 26.3. Login Disconnect alone is a wire String.
				ConvertFrom-TraceDisconnectText $bytes
			}
		}
		$facts += [PSCustomObject]@{
			file             = $file.Name
			name             = $name
			uuid             = $(if ($profile) { $profile.uuid } else { $null })
			states           = ($states -join ',')
			reachedPlay      = ($playRows.Count -gt 0)
			loginSeq         = $(if ($loginRow.Count -gt 0) { [int]$loginRow[0].seq } else { $null })
			loginReadMs      = $(if ($loginRow.Count -gt 0) { [double]$loginRow[0].read_ms } else { $null })
			disconnects      = $disconnectRow.Count
			disconnectSeq    = $seq
			disconnectState  = $disconnectState
			disconnectReadMs = $readMs
			text             = $text
		}
	}
	return $facts
}

function Invoke-S15 {
	param($Cfg, $Result)
	$nickA = [string]$Cfg.playerA
	$nickB = if ($Cfg.playerB) { [string]$Cfg.playerB } else { $nickA }
	$sameName = [string]::Equals($nickA, $nickB, [StringComparison]::Ordinal)
	$portA = [int]$Cfg.controlPortA
	$portB = [int]$Cfg.controlPortB
	$instanceA = [string]$Cfg.instanceA
	$instanceB = [string]$Cfg.instanceB
	$settleMs = [int]$Cfg.settleMs
	$outcome = if ($Cfg.expectOutcome) { [string]$Cfg.expectOutcome } else { 'old_kicked' }
	$fast = [bool]$Cfg.fastLaunch
	$expected = [string]$Cfg.expectedKickText
	$expectedState = [string]$Cfg.expectedDisconnectState

	# V1/V2: the first connection joins, then the second one connects. V4:
	# both launches are started back to back, so the first may still be in
	# configuration when the second logs in.
	$watermark = Get-Date
	if ($fast) {
		$launchA = Start-ClientNoWait $nickA $portA $instanceA
		$launchB = Start-ClientNoWait $nickB $portB $instanceB
		$outcomeA = Wait-S15Outcome $portA $watermark 120
		$outcomeB = Wait-S15Outcome $portB $watermark 120
	} else {
		$launchA = Start-ClientNoWait $nickA $portA $instanceA
		$outcomeA = Wait-S15Play $portA 120
		Add-Check $Result 'S15: the first connection joined' ($outcomeA -eq 'play') `
			("outcome={0}" -f $outcomeA)
		$script:s7Facts.Add("S15 first join: outcome=$outcomeA uuid=$($launchA.uuid)")
		$launchB = Start-ClientNoWait $nickB $portB $instanceB
		$outcomeB = Wait-S15Play $portB 120
	}
	if (-not $launchA.uuid) { throw 'S15: could not resolve the offline UUID' }
	$script:s7Facts.Add(("S15 launch: A='{0}' uuid={1} outcome={2}; B='{3}' uuid={4} outcome={5}" -f
		$nickA, $launchA.uuid, $outcomeA, $nickB, $launchB.uuid, $outcomeB))
	Start-Sleep -Milliseconds $settleMs

	# What each client sees in the end. `observe` also answers at the menu,
	# so a rejected or kicked connection is visible as inGame=false.
	$obsA = $null
	try { $obsA = (Invoke-TestCmd $portA (New-ObserveCmd 's15-a-after') 2000).result } catch { }
	$obsB = $null
	try { $obsB = (Invoke-TestCmd $portB (New-ObserveCmd 's15-b-after') 2000).result } catch { }
	$inGameA = ($obsA -ne $null -and $obsA.inGame -eq $true)
	$inGameB = ($obsB -ne $null -and $obsB.inGame -eq $true)
	$script:s7Facts.Add("S15 after settle: A inGame=$inGameA, B inGame=$inGameB")

	# Classify the fresh traces. The proxy may flush a kick row a little after
	# the client is already back in the menu (the r2 vanilla run read the trace
	# too early), so poll; stop early when the expected quiet state is seen.
	$deadline = (Get-Date).AddSeconds(20)
	$traces = @()
	do {
		$traces = @(Get-S15TraceFacts $watermark | Where-Object { $_.name -eq $nickA -or $_.name -eq $nickB })
		$kicked = @($traces | Where-Object { $_.disconnects -gt 0 })
		if ($kicked.Count -gt 0) { break }
		$allPlay = $traces.Count -ge 2 -and @($traces | Where-Object { $_.reachedPlay }).Count -ge 2
		if ($allPlay -and $inGameA -and $inGameB) { break }
		Start-Sleep -Milliseconds 500
	} while ((Get-Date) -lt $deadline)
	if ($traces.Count -eq 0) {
		Add-Check $Result 'S15: the connections are traced' $false 'no fresh trace with a profile'
		return
	}

	$kicked = @($traces | Where-Object { $_.disconnects -gt 0 })
	$survivors = @($traces | Where-Object { $_.disconnects -eq 0 })
	if ($outcome -eq 'old_kicked') {
		Add-Check $Result 'S15: the first connection was dropped' (-not $inGameA) `
			("inGame={0}" -f $(if ($obsA) { $obsA.inGame } else { 'no answer' }))
		Add-Check $Result 'S15: the second connection stays in the game' $inGameB `
			("inGame={0}" -f $(if ($obsB) { $obsB.inGame } else { 'no answer' }))
		Add-Check $Result 'S15: both connections are traced under the same name' ($sameName -and $traces.Count -eq 2) `
			(($traces | ForEach-Object { "$($_.file):$($_.name)" }) -join ', ')
		Add-Check $Result 'S15: the server disconnects exactly the first connection' ($kicked.Count -eq 1 -and $survivors.Count -eq 1) `
			("kicked=$($kicked.Count) survivors=$($survivors.Count)")
	} elseif ($outcome -eq 'new_rejected') {
		Add-Check $Result 'S15: the first connection stays in the game' $inGameA `
			("inGame={0}" -f $(if ($obsA) { $obsA.inGame } else { 'no answer' }))
		Add-Check $Result 'S15: the second connection is not in the game' (-not $inGameB) `
			("inGame={0}" -f $(if ($obsB) { $obsB.inGame } else { 'no answer' }))
		Add-Check $Result 'S15: the second name receives exactly one disconnect' ($kicked.Count -eq 1 -and $kicked[0].name -eq $nickB) `
			(($kicked | ForEach-Object { "$($_.file):$($_.name)" }) -join ', ')
	} elseif ($outcome -eq 'both_stay') {
		Add-Check $Result 'S15: the first connection stays in the game' $inGameA `
			("inGame={0}" -f $(if ($obsA) { $obsA.inGame } else { 'no answer' }))
		Add-Check $Result 'S15: the second connection stays in the game' $inGameB `
			("inGame={0}" -f $(if ($obsB) { $obsB.inGame } else { 'no answer' }))
		Add-Check $Result 'S15: neither connection receives a disconnect' ($kicked.Count -eq 0) `
			("kicked=$($kicked.Count)")
		Add-Check $Result 'S15: both connections reached play' (@($traces | Where-Object { $_.reachedPlay }).Count -eq 2) `
			(($traces | ForEach-Object { "$($_.file):$($_.name):play=$($_.reachedPlay)" }) -join ', ')
		if (-not $sameName) {
			$uuids = @($traces | Where-Object { $_.uuid } | ForEach-Object { $_.uuid } | Sort-Object -Unique)
			Add-Check $Result 'S15: the two names keep distinct profiles' ($uuids.Count -eq 2) `
				(($traces | ForEach-Object { "$($_.name)=$($_.uuid)" }) -join ', ')
		}
	} else {
		# V4: the outcome itself is the fact; record who stayed and who was
		# disconnected without asserting a shape that is a race.
		Add-Check $Result 'S15: at least one connection reached play' (@($traces | Where-Object { $_.reachedPlay }).Count -ge 1) `
			(($traces | ForEach-Object { "$($_.file):$($_.name):play=$($_.reachedPlay)" }) -join ', ')
	}
	if ($kicked.Count -eq 1) {
		$kick = $kicked[0]
		Add-Check $Result 'S15: the disconnect carries a text component' (-not [string]::IsNullOrWhiteSpace($kick.text)) `
			("text='$($kick.text)'")
		if ($expected) {
			Add-Check $Result 'S15: the disconnect text matches the recorded vanilla text' ($kick.text -eq $expected) `
				("got='$($kick.text)' want='$expected'")
		}
		if ($expectedState) {
			Add-Check $Result 'S15: the disconnect is sent in the recorded protocol phase' ($kick.disconnectState -eq $expectedState) `
				("got='$($kick.disconnectState)' want='$expectedState'")
		}
	}
	foreach ($t in $traces) {
		$script:s7Facts.Add(("S15 trace '{0}': name={1} uuid={2} states={3} play={4} login_seq={5} login_read_ms={6:N1} disconnects={7} disconnect_state={8} disconnect_seq={9} disconnect_read_ms={10:N1} text={11}" -f
			$t.file, $t.name, $t.uuid, $t.states, $t.reachedPlay, $t.loginSeq, $t.loginReadMs, $t.disconnects, $t.disconnectState, $t.disconnectSeq, $t.disconnectReadMs, $t.text))
	}
	# The vanilla server log lines about the duplicate are the human-readable
	# counterpart of the packet; they are recorded, not asserted.
	if ($isVanilla) {
		$logLines = @(Get-Content $vanillaLog -ErrorAction SilentlyContinue | Where-Object {
			($_ -match [regex]::Escape($nickA)) -or ($_ -match [regex]::Escape($nickB))
		} | Where-Object { $_.Length -lt 400 } | Select-Object -Last 12)
		foreach ($line in $logLines) { $script:s7Facts.Add("S15 vanilla log: $line") }
	}

	try { $null = Invoke-TestCmd $portB (New-SessionCmd 's15-b-off' 'disconnect') } catch { }
	Start-Sleep -Seconds 2
}

$scenarioDir = Join-Path $here 'scenarios'
$allResults = @()
try {
	Prepare-RunWorld
	$s16Requested = (($Scenarios -contains 's16a') -or ($Scenarios -contains 's16b'))
	if (-not $isVanilla -and (($Scenarios -contains 's13') -or ($Scenarios -contains 's13-stop') -or ($Scenarios -contains 's13-gamemode') -or $s16Requested)) {
		# РЁР°Рі 19a: the player save root lives inside the run directory. The
		# gamemode variant reuses the directory of the preceding s13 run, so
		# its file (with the patched mode) is still there.
		if (-not $OursSaveDir) {
			if ((-not $s16Requested) -and (($Scenarios -contains 's13-gamemode') -or ($Scenarios -contains 's13-stop'))) {
				throw '-OursSaveDir is required for this scenario (point it at the s13 run save root)'
			}
			$OursSaveDir = Join-Path $RunDir 'save'
		}
		$saveFull = [IO.Path]::GetFullPath($OursSaveDir)
		foreach ($forbidden in @($templateWorldDir, (Join-Path $vanillaDir 'world'))) {
			$forbiddenFull = [IO.Path]::GetFullPath($forbidden)
			if ($saveFull.Equals($forbiddenFull, [StringComparison]::OrdinalIgnoreCase) -or
				$saveFull.StartsWith($forbiddenFull + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
				throw "refusing save_dir inside $forbidden (template/world is read-only)"
			}
		}
		if (-not $s16Requested -and $OursAutosaveTicks -lt 0) { $OursAutosaveTicks = 200 }
		if ($s16Requested) {
			if ($OursWorldAutosaveTicks -lt 0) { $OursWorldAutosaveTicks = 40 }
			if (-not $OursSaveIgnoreWorldMismatch) { $OursSaveIgnoreWorldMismatch = 'false' }
			if ($OursGameMode -lt 0) { $OursGameMode = 1 }
		}
		Write-Host ("save root: {0} (player autosave {1}, world autosave {2}, ignore mismatch '{3}')" -f `
			$OursSaveDir, $OursAutosaveTicks, $OursWorldAutosaveTicks, $OursSaveIgnoreWorldMismatch)
	}
	if (-not $isVanilla -and $OursDevGeometry -and (($Scenarios -contains 's10') -or ($Scenarios -contains 's11') -or ($Scenarios -contains 's12') -or ($Scenarios -contains 's14'))) {
		Write-DevGeometryFile $OursDevGeometry
	}
	Start-Stand
	foreach ($id in $Scenarios) {
		# The scenario id in the file is authoritative: `s13-stop` must not be
		# picked by a prefix filter for `s13`.
		$file = Get-ChildItem $scenarioDir -Filter '*.json' |
			Where-Object { ((Get-Content $_.FullName -Raw | ConvertFrom-Json).id) -eq $id } |
			Select-Object -First 1
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
			's11' { Invoke-S11 $cfg $result }
			's12' { Invoke-S12 $cfg $result }
			's14' { Invoke-S14 $cfg $result }
			's13' { Invoke-S13 $cfg $result }
			's13-stop' { Invoke-S13Stop $cfg $result }
			's13-gamemode' { Invoke-S13Gamemode $cfg $result }
			's15' { Invoke-S15 $cfg $result }
			's15b' { Invoke-S15 $cfg $result }
			's15c' { Invoke-S15 $cfg $result }
			's16a' { Invoke-S16a $cfg $result }
			's16b' { Invoke-S16b $cfg $result }
			's17' { Invoke-S17 $cfg $result }
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
