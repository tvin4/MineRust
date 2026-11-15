$ErrorActionPreference = 'Stop'

# Parses run-scenario.ps1 and exercises its pure S5 trace helpers against the
# byte payloads recorded in real runs. Nothing here touches E:\MineRust-data;
# payloads are inlined as the hex strings the traces contain.
$runnerPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'run-scenario.ps1'
$tokens = $null
$parseErrors = $null
[System.Management.Automation.Language.Parser]::ParseFile($runnerPath, [ref]$tokens, [ref]$parseErrors) | Out-Null
if ($parseErrors.Count -gt 0) {
	$messages = $parseErrors | ForEach-Object { "line $($_.Extent.StartLineNumber): $($_.Message)" }
	throw "run-scenario.ps1 parse failed: $($messages -join '; ')"
}

$source = Get-Content -LiteralPath $runnerPath -Raw
$start = $source.IndexOf('function Select-SprintAttribute {')
$end = $source.IndexOf('function Wait-ObserverChunksStable {')
if ($start -lt 0 -or $end -le $start) { throw 'Select-SprintAttribute not found' }
Invoke-Expression $source.Substring($start, $end - $start)

function New-Attribute {
	param([int]$Seq, [string]$Hex, [bool]$Sprint)
	[PSCustomObject]@{ seq = $Seq; hex = $Hex; sprint = $Sprint }
}

# Real payloads from run 2026-11-15_1908 (proxy-conn-2.jsonl).
$on1908 = New-Attribute 849 '01 01 1a 3f b9 99 99 a0 00 00 00 01 13 6d 69 6e 65 63 72 61 66 74 3a 73 70 72 69 6e 74 69 6e 67 3f d3 33 33 40 00 00 00 02' $true
# Real payloads from the green run 2026-11-15_step10-live-ours-s5-observe.
$onGreen = New-Attribute 905 '01 01 1a 3f b9 99 99 a0 00 00 00 01 13 6d 69 6e 65 63 72 61 66 74 3a 73 70 72 69 6e 74 69 6e 67 3f d3 33 33 40 00 00 00 02' $true
$offGreen = New-Attribute 1212 '01 01 1a 3f b9 99 99 a0 00 00 00 00' $false

# 1. Meeting the sprint-on payload must be found even when the release flag was
#    never captured: this is the exact 1908 failure that read "missing".
$selection = Select-SprintAttribute @($on1908) 847 $true
if (-not $selection.found -or $selection.seq -ne 849) {
	throw "sprint-on attribute was not selected without a release anchor (found=$($selection.found) reason=$($selection.reason))"
}
$expectedOn = '01 01 1a 3f b9 99 99 a0 00 00 00 01 13 6d 69 6e 65 63 72 61 66 74 3a 73 70 72 69 6e 74 69 6e 67 3f d3 33 33 40 00 00 00 02'
if ($selection.hex -ne $expectedOn) { throw 'sprint-on payload hex does not match the recorded bytes' }

# 2. An attribute that precedes its anchor flag is not a match.
$early = Select-SprintAttribute @($onGreen) 906 $true
if ($early.found) { throw 'an attribute before the anchor flag must not be selected' }

# 3. The green sequence resolves both transitions.
$onOk = Select-SprintAttribute @($onGreen, $offGreen) 904 $true
$offOk = Select-SprintAttribute @($onGreen, $offGreen) 1211 $false
if (-not $onOk.found -or $onOk.seq -ne 905) { throw 'green sprint-on attribute was not selected' }
if (-not $offOk.found -or $offOk.seq -ne 1212) { throw 'green sprint-off attribute was not selected' }

# 4. A missing anchor is reported as such, never as a missing payload.
$noAnchor = Select-SprintAttribute @($on1908) $null $true
if ($noAnchor.found -or $noAnchor.reason -notmatch 'anchor metadata flag was not captured') {
	throw 'a missing anchor must be reported as a missing anchor'
}

# 5. An absent release record names the missing transition.
$noOff = Select-SprintAttribute @($on1908) 847 $false
if ($noOff.found -or $noOff.reason -notmatch 'no sprint-off attribute record after seq 847') {
	throw 'a missing sprint-off record must name the transition and the anchor'
}

# 6. The observer timeline: a late sprint-off counts, the +0 ms sample alone
#    does not. Run 2026-11-15_s5-live-clean reported isSprinting=true at +8 ms
#    and false at +159 ms; a check keyed to the +0 ms sample alone failed there.
function New-Observation {
	param([int]$Target, [double]$Actual, [bool]$Found, [object]$Sprinting)
	[PSCustomObject]@{ targetAfterReleaseMs = $Target; actualAfterReleaseMs = $Actual; found = $Found; isSprinting = $Sprinting }
}

$lateOnly = Get-SprintOffObservation @(
	(New-Observation 0 8 $true $true),
	(New-Observation 100 159 $true $false)
)
if (-not $lateOnly.observed) { throw 'a sprint-off at a later sample must count as observed' }
if ($lateOnly.timeline -notmatch '\+0/8ms=True/True' -or $lateOnly.timeline -notmatch '\+100/159ms=True/False') {
	throw "the timeline must keep every sample: $($lateOnly.timeline)"
}

$firstOnly = Get-SprintOffObservation @( (New-Observation 0 8 $true $true) )
if ($firstOnly.observed) { throw 'the +0 ms sample alone must not satisfy the check' }

# 7. A never-found observer never satisfies the check.
$neverFound = Get-SprintOffObservation @(
	(New-Observation 0 5 $false $null),
	(New-Observation 100 104 $false $null)
)
if ($neverFound.observed) { throw 'an observer that never found the entity must not satisfy the check' }

Write-Host 'S5 attribute-selection tests passed.'
