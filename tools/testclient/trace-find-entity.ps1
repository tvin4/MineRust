# tools/testclient - stage-10 trace probe (throwaway analysis helper).
#
# Finds the entity id whose add_entity carries the given UUID prefix, then
# prints every packet for that entity id (set_entity_data etc) with seq order.
param(
	[Parameter(Mandatory = $true)][string]$Path,
	[Parameter(Mandatory = $true)][string]$UuidHex
)
$ErrorActionPreference = 'Stop'
$rows = Get-Content $Path -ReadCount 2000 | ForEach-Object { $_ } | Where-Object { $_.Trim() } |
	ForEach-Object { $_ | ConvertFrom-Json }
foreach ($r in $rows) {
	if ($r.name -ne 'minecraft:add_entity' -or $r.state -ne 'play') { continue }
	$bytes = [Convert]::FromBase64String($r.b64)
	$hex = ($bytes | ForEach-Object { $_.ToString('x2') }) -join ' '
	if ($hex -notmatch [regex]::Escape($UuidHex)) { continue }
	$off = 0
	function Read-VarintX($b, [ref]$p) { $v = 0; $s = 0; do { $c = $b[$p.Value]; $p.Value++; $v = $v -bor (($c -band 0x7f) -shl $s); $s += 7 } while ($c -band 0x80); return $v }
	$id = Read-VarintX $bytes ([ref]$off)
	"entity_id=$id seq=$($r.seq)"
	$eidHex = ($id -band 0x7f) -bor 0x80
	foreach ($q in $rows) {
		if ($q.state -ne 'play' -or $q.dir -ne 'clientbound') { continue }
		$b2 = [Convert]::FromBase64String($q.b64)
		if ($b2.Length -lt 1) { continue }
		$o2 = 0
		$vid = Read-VarintX $b2 ([ref]$o2)
		if ($vid -ne $id) { continue }
		if ($q.name -notmatch 'set_entity_data|update_attributes|move_entity|rotate_head|teleport|entity_position|animate|swing') { continue }
		$hex2 = ($b2 | ForEach-Object { $_.ToString('x2') }) -join ' '
		'{0,6} {1,-12} {2,-40} {3}' -f $q.seq, $q.dir, $q.name, $hex2
	}
	break
}
