$ErrorActionPreference = 'Stop'

$toolDir = Split-Path -Parent $PSScriptRoot
$runnerPath = Join-Path $toolDir 'run-scenario.ps1'
. (Join-Path $toolDir 'scenario-lib.ps1')

$tokens = $null
$parseErrors = $null
[System.Management.Automation.Language.Parser]::ParseFile($runnerPath, [ref]$tokens, [ref]$parseErrors) | Out-Null
if ($parseErrors.Count -gt 0) {
	$messages = $parseErrors | ForEach-Object { "line $($_.Extent.StartLineNumber): $($_.Message)" }
	throw "run-scenario.ps1 parse failed: $($messages -join '; ')"
}

$block = New-BlockCmd 'block-test' 98 135 -32 | ConvertFrom-Json
if ($block.cmd -ne 'block' -or $block.args.x -ne 98 -or $block.args.y -ne 135 -or $block.args.z -ne -32) {
	throw 'New-BlockCmd did not preserve its coordinate'
}
$dig = New-DigPacketCmd 'dig-test' 90 60 -7 1 | ConvertFrom-Json
if ($dig.cmd -ne 'digpacket' -or $dig.args.x -ne 90 -or $dig.args.k -ne 1) {
	throw 'New-DigPacketCmd did not preserve its coordinate/tick count'
}

# Load only the pure CRC checker, not the runner's process-launching body.
$source = Get-Content -LiteralPath $runnerPath -Raw
$start = $source.IndexOf('function Test-S16MrChunk {')
$end = $source.IndexOf('function Get-S16SaveAudit {')
if ($start -lt 0 -or $end -le $start) { throw 'S16 CRC helper not found' }
Invoke-Expression $source.Substring($start, $end - $start)

$root = Join-Path $env:TEMP ('minerust-s16-runner-test-' + [guid]::NewGuid().ToString('N'))
try {
	New-Item -ItemType Directory -Path $root | Out-Null
	$file = Join-Path $root '2.-3.mrchunk'
	$hex = '4d5253560102000000fdffffff010000002a000000012fbed932'
	[byte[]]$bytes = (0..(($hex.Length / 2) - 1) | ForEach-Object {
		[Convert]::ToByte($hex.Substring($_ * 2, 2), 16)
	})
	[IO.File]::WriteAllBytes($file, $bytes)
	$valid = Test-S16MrChunk $file
	if (-not $valid.valid) { throw "known .mrchunk fixture failed CRC: $($valid.detail)" }
	$bytes[17] = $bytes[17] -bxor 1
	[IO.File]::WriteAllBytes($file, $bytes)
	$damaged = Test-S16MrChunk $file
	if ($damaged.valid) { throw 'damaged .mrchunk fixture passed CRC' }
} finally {
	if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}

Write-Host 'S16 runner command/CRC tests passed.'

# Read-TraceMatches (streaming trace filter, used by S17 post-processing) must
# keep only the requested packet rows and survive a torn trailing line.
$tracePath = Join-Path $env:TEMP ('minerust-trace-matches-' + [guid]::NewGuid().ToString('N') + '.jsonl')
try {
	$lines = @(
		'{"name":"minecraft:keep_alive","dir":"clientbound","seq":1}',
		'{"name":"minecraft:use_item_on","dir":"serverbound","seq":2}',
		'{"name":"minecraft:move_player_pos","dir":"serverbound","seq":3}',
		'{"name":"minecraft:pick_item_from_block","dir":"serverbound","seq":4}',
		'{"name":"minecraft:use_item_on","dir":"serverbound","seq":5,"b64":"torn'
	)
	[IO.File]::WriteAllLines($tracePath, $lines)
	$rows = @(Read-TraceMatches $tracePath @('minecraft:use_item_on', 'minecraft:pick_item_from_block'))
	if ($rows.Count -ne 2) { throw "Read-TraceMatches kept $($rows.Count) rows, expected 2 (torn line skipped)" }
	if (@($rows | Where-Object { $_.name -eq 'minecraft:use_item_on' }).Count -ne 1) {
		throw 'Read-TraceMatches dropped a wanted use_item_on row'
	}
	if (@($rows | Where-Object { $_.name -eq 'minecraft:keep_alive' }).Count -ne 0) {
		throw 'Read-TraceMatches kept an unwanted row'
	}
} finally {
	if (Test-Path -LiteralPath $tracePath) { Remove-Item -LiteralPath $tracePath -Force }
}
Write-Host 'Read-TraceMatches test passed.'
