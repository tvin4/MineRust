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
#   .\run-scenario.ps1 -KeepStand         # leave vanilla+mcproxy running
#
# Results: <runDir>\summary.json, traces under <runDir>\mcproxy.
# The stand is always torn down in finally (own vanilla/mcproxy/clients).
[CmdletBinding()]
param(
	[string[]]$Scenarios = @('s1', 's2', 's3', 's4'),
	[string]$RunDir = '',
	[switch]$KeepStand
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'scenario-lib.ps1')

# --- paths -----------------------------------------------------------------
$vanillaDir = 'E:\MineRust-data\vanilla'
$serverJar = 'E:\MineRust-data\server.jar'
$mcproxyExe = 'E:\projects\MineRust\target\debug\mcproxy.exe'
if (-not $RunDir) { $RunDir = Join-Path 'E:\MineRust-data\autotests' (Get-Date -Format 'yyyy-MM-dd_HHmm') }
New-Item -ItemType Directory -Path $RunDir -Force | Out-Null
$traceDir = Join-Path $RunDir 'mcproxy'
New-Item -ItemType Directory -Path $traceDir -Force | Out-Null
$vanillaLog = Join-Path $RunDir 'vanilla-stdout.log'
$rconScript = Join-Path $here 'Invoke-Rcon.ps1'
$launchScript = Join-Path $here 'launch-fabric-client.ps1'
$summaryPath = Join-Path $RunDir 'summary.json'

# Stand PIDs, cleaned in finally.
$script:vanillaPid = $null
$script:mcproxyPid = $null

function Start-Stand {
	Start-Process java -WorkingDirectory $vanillaDir `
		-ArgumentList '-Xmx1G', '-jar', $serverJar, 'nogui' `
		-RedirectStandardOutput $vanillaLog `
		-RedirectStandardError (Join-Path $RunDir 'vanilla-stderr.log') -PassThru |
		ForEach-Object { $script:vanillaPid = $_.Id }
	for ($i = 0; $i -lt 60; $i++) {
		if (Get-NetTCPConnection -LocalPort 25566 -State Listen -ErrorAction SilentlyContinue) { break }
		Start-Sleep -Seconds 1
	}
	if (-not (Get-NetTCPConnection -LocalPort 25566 -State Listen -ErrorAction SilentlyContinue)) {
		throw 'vanilla did not open 25566'
	}
	for ($i = 0; $i -lt 30; $i++) {
		if (Get-NetTCPConnection -LocalPort 25575 -State Listen -ErrorAction SilentlyContinue) { break }
		Start-Sleep -Seconds 1
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
	if (-not (Wait-FreshJoin -Log $vanillaLog -Nick $Nick -Since $Since -TimeoutSec $TimeoutSec)) {
		throw "$Nick did not join in ${TimeoutSec}s"
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
	& $rconScript -Command 'save-all flush' | Out-Null
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
	& $rconScript -Command 'save-all flush' | Out-Null
	Start-Sleep -Seconds 1
	$nbtPath = Get-PlayerDataPath (Join-Path $vanillaDir 'world') $launchA.uuid
	$rot = $null
	if ($nbtPath -and (Test-Path $nbtPath)) { $rot = Get-PlayerRotationFromNbt $nbtPath }
	$nbtHit = ($rot -and [Math]::Abs($rot.yaw - [float]$saved.yaw) -le $tol -and [Math]::Abs($rot.pitch - [float]$saved.pitch) -le $tol)
	$newTraces = Get-NewTraceFiles $traceDir $relaunchAt
	$traceHit = $false
	foreach ($trace in $newTraces) {
		if (Find-RotationInTrace (Read-Trace $trace.FullName) ([float]$saved.yaw) ([float]$saved.pitch) $tol) { $traceHit = $true }
	}
	# The owner condition is "NBT OR the relaunch trace": either is enough.
	$nbtFact = if ($rot) { "yaw=$($rot.yaw) pitch=$($rot.pitch)" } else { 'no NBT file' }
	Add-Check $Result 'saved rotation present in playerdata (NBT or relaunch trace)' ($nbtHit -or $traceHit) `
		("NBT {0}: {1} -> {2}; relaunch trace -> {3} (saved yaw={4} pitch={5})" -f `
			$nbtPath, $nbtFact, $(if ($nbtHit) { 'hit' } else { 'miss' }), `
			$(if ($traceHit) { 'hit' } else { 'miss' }), $saved.yaw, $saved.pitch)
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
		& $rconScript -Command ("gamemode {0} {1}" -f $mode, $nickA) | Out-Null
		Start-Sleep -Milliseconds $settle
		$seen = (Invoke-TestCmd $portB (New-ObserveCmd 's4-obs' $nickA)).result
		Add-Check $Result ("B sees A in {0} after RCON gamemode" -f $mode) ($seen.gameMode -eq $mode) `
			("gameMode seen by B: {0} (expected {1})" -f $seen.gameMode, $mode)
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
			's2' { Invoke-S2 $cfg $result }
			's3' { Invoke-S3 $cfg $result }
			's4' { Invoke-S4 $cfg $result }
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
	generated = (Get-Date).ToString('s')
	scenarios = $allResults
}
$summary | ConvertTo-Json -Depth 6 | Set-Content $summaryPath -Encoding utf8
Write-Host "summary: $summaryPath"
$failedCount = @($allResults | Where-Object { $_.failed }).Count
Write-Host ("{0} scenario(s) run, {1} with failed checks" -f $allResults.Count, $failedCount)
if ($failedCount -gt 0) { exit 1 }
