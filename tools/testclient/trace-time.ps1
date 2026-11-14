# tools/testclient - stage-10 trace probe (throwaway analysis helper).
#
# Prints seq, wall-clock ms and payload for rows matching a name pattern,
# so two traces can be correlated by time instead of by seq.
param(
	[Parameter(Mandatory = $true)][string]$Path,
	[Parameter(Mandatory = $true)][string]$Name
)
$ErrorActionPreference = 'Stop'
$rows = Get-Content $Path -ReadCount 2000 | ForEach-Object { $_ } | Where-Object { $_.Trim() } |
	ForEach-Object { $_ | ConvertFrom-Json }
foreach ($r in $rows) {
	if ($r.name -notmatch $Name) { continue }
	$bytes = [Convert]::FromBase64String($r.b64)
	$hex = ($bytes | ForEach-Object { $_.ToString('x2') }) -join ' '
	'{0,6} {1,-12} {2,12} {3,12} {4,-30} {5}' -f $r.seq, $r.dir, $r.read_ms, $r.write_ms, $r.name, $hex
}
