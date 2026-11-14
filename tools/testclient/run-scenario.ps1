# tools/testclient - scenario orchestrator (stage V).
#
# Runs the visibility scenarios S1-S4 against the vanilla test stand:
# vanilla server (E:\MineRust-data\vanilla, RCON 25575) behind mcproxy
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
# Results: <runDir>\summary.json, traces under <runDir>\mcproxy.
# The stand is always torn down in finally (own vanilla/mcproxy/clients).
[CmdletBinding()]
param(
	[string[]]$Scenarios = @('s1', 's1b', 's2', 's3', 's4', 's5'),
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
$vanillaLog = Join-Path $RunDir 'vanilla-stdout.log'
$oursLog = Join-Path $RunDir 'ours-stdout.log'
$rconScript = Join-Path $here 'Invoke-Rcon.ps1'
$launchScript = Join-Path $here 'launch-fabric-client.ps1'
$summaryPath = Join-Path $RunDir 'summary.json'

# Stand PIDs, cleaned in finally.
$script:vanillaPid = $null
$script:oursPid = $null
$script:mcproxyPid = $null

function Start-Stand {
	if ($isVanilla) {
		Start-Process java -WorkingDirectory $vanillaDir `
			-ArgumentList '-Xmx1G', '-jar', $serverJar, 'nogui' `
			-RedirectStandardOutput $vanillaLog `
			-RedirectStandardError (Join-Path $RunDir 'vanilla-stderr.log') -PassThru |
			ForEach-Object { $script:vanillaPid = $_.Id }
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
	for ($i = 0; $i -lt 60; $i++) {
		if (Get-NetTCPConnection -LocalPort 25566 -State Listen -ErrorAction SilentlyContinue) { break }
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
	if ($script:mcproxyPid) { Stop-Process -Id $script:mcproxyPid -Force -ErrorAction SilentlyContinue }
	if ($script:vanillaPid) { Stop-Process -Id $script:vanillaPid -Force -ErrorAction SilentlyContinue }
	if ($script:oursPid) { Stop-Process -Id $script:oursPid -Force -ErrorAction SilentlyContinue }
}

# One RCON command on the vanilla console. Our server has no RCON (stage G):
# the call is skipped there and the scenario records the gap instead.
function Invoke-StandRcon {
	param([string]$Command)
	if ($isVanilla) { & $rconScript -Command $Command | Out-Null }
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
	$uuidBytes = ([guid]::Parse($launchA.uuid)).ToByteArray()
	$spawn = $bRows | Where-Object {
		if ($_.dir -ne 'clientbound' -or $_.name -ne 'minecraft:add_entity') { return $false }
		$payload = ConvertFrom-TraceB64 $_.b64
		if ($payload.Length -lt 17) { return $false }
		for ($i = 0; $i -lt 16; $i++) { if ($payload[$i + 1] -ne $uuidBytes[$i]) { return $false } }
		return $true
	} | Select-Object -First 1
	$entityId = $null
	if ($spawn) { $spawnPayload = ConvertFrom-TraceB64 $spawn.b64; if ($spawnPayload[0] -lt 0x80) { $entityId = [int]$spawnPayload[0] } }
	if ($null -eq $entityId) {
		Add-Check $Result 'S5 resolves A entity id from B add_entity' $false 'missing spawn or entity id is not a one-byte VarInt'
		return
	}

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
	# The attribute payload's head is the entity's runtime id (a one-byte
	# VarInt here); the rest is the traced vanilla payload of the sprinting
	# attribute (S5 trace: 21 01 1a ...).
	$onHex = ('{0:x2} ' -f $entityId) + '01 1a 3f b9 99 99 a0 00 00 00 01 13 6d 69 6e 65 63 72 61 66 74 3a 73 70 72 69 6e 74 69 6e 67 3f d3 33 33 40 00 00 00 02'
	$offHex = ('{0:x2} ' -f $entityId) + '01 1a 3f b9 99 99 a0 00 00 00 00'
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
Write-Host "summary: $summaryPath"
$failedCount = @($allResults | Where-Object { $_.failed }).Count
Write-Host ("{0} scenario(s) run, {1} with failed checks" -f $allResults.Count, $failedCount)
if ($failedCount -gt 0) { exit 1 }
