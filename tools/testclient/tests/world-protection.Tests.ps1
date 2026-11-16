$ErrorActionPreference = 'Stop'

$runnerPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'run-scenario.ps1'
$tokens = $null
$parseErrors = $null
[System.Management.Automation.Language.Parser]::ParseFile($runnerPath, [ref]$tokens, [ref]$parseErrors) | Out-Null
if ($parseErrors.Count -gt 0) {
	$messages = $parseErrors | ForEach-Object { "line $($_.Extent.StartLineNumber): $($_.Message)" }
	throw "run-scenario.ps1 parse failed: $($messages -join '; ')"
}

$source = Get-Content -LiteralPath $runnerPath -Raw
$start = $source.IndexOf('function Assert-WorldPlayerData {')
$end = $source.IndexOf('function Start-Stand {')
if ($start -lt 0 -or $end -le $start) { throw 'world protection functions not found' }
Invoke-Expression $source.Substring($start, $end - $start)

Assert-WorldPlayerData @('known.dat') @('known.dat') | Out-Null
$blocked = $false
try {
	Assert-WorldPlayerData @('alice.dat') @('template-player.dat') | Out-Null
} catch {
	$blocked = $_.Exception.Message -match 'playerdata absent from template.*alice\.dat'
}
if (-not $blocked) { throw 'unexpected playerdata was not rejected with a clear error' }

$root = Join-Path $env:TEMP ('minerust-world-audit-test-' + [guid]::NewGuid().ToString('N'))
try {
	$template = Join-Path $root 'world-template-v3'
	$vanilla = Join-Path $root 'vanilla'
	$runDir = Join-Path $root 'run'
	New-Item -ItemType Directory -Path (Join-Path $template 'players\data') -Force | Out-Null
	New-Item -ItemType Directory -Path (Join-Path $vanilla 'world\players\data') -Force | Out-Null
	New-Item -ItemType Directory -Path $runDir -Force | Out-Null
	Set-Content -LiteralPath (Join-Path $template 'level.dat') -Value 'template'
	Set-Content -LiteralPath (Join-Path $vanilla 'world\level.dat') -Value 'old world'
	Set-Content -LiteralPath (Join-Path $vanilla 'world\players\data\alice.dat') -Value 'preserve me'

	$source = Get-Content -LiteralPath $runnerPath -Raw
	$start = $source.IndexOf('function Get-WorldSnapshot {')
	$end = $source.IndexOf('function Start-Stand {')
	Invoke-Expression $source.Substring($start, $end - $start)
	$script:templateWorldDir = $template
	$script:vanillaDir = $vanilla
	$script:RunDir = $runDir
	$script:isVanilla = $true
	$script:Target = 'vanilla'
	Prepare-RunWorld

	$active = Join-Path $vanilla 'world'
	if (-not (Test-Path -LiteralPath (Join-Path $active 'level.dat'))) { throw 'template was not copied into the active world' }
	$stale = @(Get-ChildItem -LiteralPath $vanilla -Directory -Filter 'world.stale-*')
	if ($stale.Count -ne 1 -or -not (Test-Path -LiteralPath (Join-Path $stale[0].FullName 'players\data\alice.dat'))) {
		throw 'old world was not preserved under world.stale-<timestamp>'
	}
	$auditPath = Join-Path $runDir 'world-audit.json'
	if (-not (Test-Path -LiteralPath $auditPath)) { throw 'world audit artifact was not written' }
	$audit = Get-Content -LiteralPath $auditPath -Raw | ConvertFrom-Json
	if ($audit.files.Count -lt 1 -or $audit.files[0].sha256 -notmatch '^[A-F0-9]{64}$') { throw 'world audit is missing SHA-256 records' }
	if ($audit.playerData.Count -ne 0 -or $audit.unexpectedPlayerData.Count -ne 0) { throw 'world audit reports unexpected playerdata after copy' }

	# Stale-world pruning (keep the two newest): two more prepares must age
	# out the first stale world (the one with the preserved alice.dat).
	Start-Sleep -Milliseconds 5
	Prepare-RunWorld
	Start-Sleep -Milliseconds 5
	Prepare-RunWorld
	$staleAfter = @(Get-ChildItem -LiteralPath $vanilla -Directory -Filter 'world.stale-*')
	if ($staleAfter.Count -ne 2) { throw ('stale worlds were not pruned to two, found: ' + $staleAfter.Count) }
	$withAlice = @($staleAfter | Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'players\data\alice.dat') })
	if ($withAlice.Count -ne 0) { throw 'the oldest stale world survived pruning' }
	if (@($staleAfter | Where-Object { -not (Test-Path -LiteralPath (Join-Path $_.FullName 'level.dat')) }).Count -ne 0) {
		throw 'a kept stale world is missing its template copy'
	}
	if (-not (Test-Path -LiteralPath (Join-Path $template 'level.dat'))) { throw 'the source template was touched by pruning' }

	$oursWorld = Join-Path $root 'world'
	New-Item -ItemType Directory -Path (Join-Path $oursWorld 'players\data') -Force | Out-Null
	Set-Content -LiteralPath (Join-Path $oursWorld 'level.dat') -Value 'old ours world'
	Set-Content -LiteralPath (Join-Path $oursWorld 'players\data\alice.dat') -Value 'keep old ours world'
	$script:isVanilla = $false
	$script:Target = 'ours'
	Prepare-RunWorld
	$oursStale = @(Get-ChildItem -LiteralPath $root -Directory -Filter 'world.stale-*')
	if ($oursStale.Count -ne 1 -or -not (Test-Path -LiteralPath (Join-Path $oursStale[0].FullName 'players\data\alice.dat'))) {
		throw 'ours world was not preserved under world.stale-<timestamp>'
	}
	if (-not (Test-Path -LiteralPath (Join-Path $oursWorld 'level.dat'))) { throw 'ours world was not recreated from the template' }
} finally {
	if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}

Write-Host 'PowerShell parse and world-protection tests passed.'
