$ErrorActionPreference = 'Stop'

# S18 restarts our server on the same save_dir. The dev geometry is applied on
# every start after the save is loaded, so re-applying the fixture on the
# restart resets the destinations to `initial` and hides what the save holds
# (2026-11-18_s18-ours-step24-r1 read air for all three cells for that reason).
# Get-PlacementGeometry -PlatformOnly must keep the stand floor and drop the
# fixture.
$toolDir = Split-Path -Parent $PSScriptRoot
$runnerPath = Join-Path $toolDir 'run-scenario.ps1'

# Load only the pure geometry helper, not the runner's process-launching body.
$source = Get-Content -LiteralPath $runnerPath -Raw
$start = $source.IndexOf('function Get-PlacementGeometry {')
$end = $source.IndexOf('function Write-DevGeometryFile {')
if ($start -lt 0 -or $end -le $start) { throw 'Get-PlacementGeometry not found' }
Invoke-Expression $source.Substring($start, $end - $start)

$cfg = [pscustomobject]@{
	cases = @(
		[pscustomobject]@{
			name      = 'stone'
			support   = @(80, 202, -20)
			dest      = @(80, 202, -21)
			initial   = 'minecraft:air'
			stand     = @(80.5, 201, -24.5, 0, 2)
		}
	)
}

$full = @(Get-PlacementGeometry $cfg)
if (@($full | Where-Object { $_ -like 'setblock*' }).Count -eq 0) {
	throw 'the full geometry lost its fixture setblock lines'
}
$platform = @(Get-PlacementGeometry $cfg -PlatformOnly)
if (@($platform | Where-Object { $_ -like 'setblock*' }).Count -ne 0) {
	throw "the restart geometry re-applies the fixture: $($platform -join '; ')"
}
if (@($platform | Where-Object { $_ -like 'fill*' }).Count -ne 1) {
	throw 'the restart geometry lost the stand floor'
}
if (@($platform | Where-Object { $_.Trim() -eq '' }).Count -ne 0) {
	throw 'the restart geometry contains an empty line'
}

Write-Host 'Placement geometry restart test passed.'