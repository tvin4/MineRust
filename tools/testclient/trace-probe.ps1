# tools/testclient - stage-10 trace probe (throwaway analysis helper).
#
# Filters one mcproxy JSONL trace and prints the packets that matter for the
# observer encoders (stage 10): the join window of a late observer, the swing
# broadcast, and the sneak/sprint metadata. One row per line: seq, state, dir,
# id, name, then the payload as hex (big-endian bytes as recorded).
param(
	[Parameter(Mandatory = $true)][string]$Path,
	[string]$Name = '',
	[switch]$PlayOnly,
	[int]$First = 0,
	[int[]]$Seqs = @()
)
$ErrorActionPreference = 'Stop'
$rows = Get-Content $Path -ReadCount 2000 | ForEach-Object { $_ } | Where-Object { $_.Trim() } |
	ForEach-Object { $_ | ConvertFrom-Json }
if ($Name) { $rows = $rows | Where-Object { $_.name -match $Name } }
if ($PlayOnly) { $rows = $rows | Where-Object { $_.state -eq 'play' } }
if ($Seqs.Count) { $rows = $rows | Where-Object { $Seqs -contains [int]$_.seq } }
if ($First -gt 0) { $rows = $rows | Select-Object -First $First }
foreach ($r in $rows) {
	$bytes = [Convert]::FromBase64String($r.b64)
	$hex = ($bytes | ForEach-Object { $_.ToString('x2') }) -join ' '
	'{0,6} {1,-13} {2,-12} id={3,-4} {4,-38} {5}' -f $r.seq, $r.state, $r.dir, $r.id, $r.name, $hex
}
