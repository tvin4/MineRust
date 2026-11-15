# Предложение: автоочистка `world.stale-*` (шаг 12, п. 7)

Статус: **предложение, не применено.** В рабочем дереве ничего не удалялось и не
менялось; сервер и тест (в) не затрагиваются. Изменение — только
`tools/testclient/run-scenario.ps1` + тест
`tools/testclient/tests/world-protection.Tests.ps1`.

## Зачем

`Prepare-RunWorld` (коммит `5eedef7`) при каждом прогоне переименовывает старый
рабочий мир в `world.stale-<yyyyMMdd-HHmmss-fff>` и никогда его не удаляет. Это
правильно для аудита (ничего не теряется), но папки копятся без предела: каждый
прогон оставляет полную копию мира (~сотни МБ). Нужен предел — оставлять
последние 2.

## Границы (жёсткие)

1. Удаляются **только** каталоги с именем, содержащим `.stale-`, и только
   **непосредственно** в каталоге, который является родителем активного мира.
2. Перед любым удалением путь приводится к полному и проверяется, что он
   **строго внутри** `E:\MineRust-data` (не равен корню, не выше него).
3. Ничего вне `E:\MineRust-data` не удаляется никогда; шаблон v3 (`world-template-v3`)
   под правило не попадает (имя без `.stale-`).
4. Удаление выполняется после успешной ротации (старая папка уже переименована),
   поэтому неудачная ротация не приводит к потере мира.
5. `-Keep` по умолчанию 2; сортировка по имени (имя содержит метку времени
   `yyyyMMdd-HHmmss-fff`, поэтому лексикографический порядок = хронологический).

## Diff (предлагаемый, не применён)

```diff
--- a/tools/testclient/run-scenario.ps1
+++ b/tools/testclient/run-scenario.ps1
@@ (после function Prepare-RunWorld)
+# Deletes all but the newest `$Keep` stale world copies under `$BaseDir`.
+#
+# Rotating a world must not lose data, so `Prepare-RunWorld` never deletes
+# and the copies pile up instead. This bounds them. Two guards run before
+# any -Recurse -Force: the base dir must be strictly inside the audited data
+# root, and only entries whose name carries `.stale-` are considered. The
+# template (`world-template-v3`) never matches.
+function Remove-OldStaleWorlds {
+	param([string]$BaseDir, [int]$Keep = 2)
+	$root = [System.IO.Path]::GetFullPath('E:\MineRust-data')
+	$base = [System.IO.Path]::GetFullPath($BaseDir)
+	$separator = [System.IO.Path]::DirectorySeparatorChar
+	if (-not $base.StartsWith($root + $separator, [StringComparison]::OrdinalIgnoreCase)) {
+		throw "refusing to clean stale worlds outside $root : $base"
+	}
+	$stale = @(Get-ChildItem -LiteralPath $base -Directory -Force -ErrorAction SilentlyContinue |
+		Where-Object {
+			$_.Name -like '*.stale-*' -and
+			$_.FullName.StartsWith($base + $separator, [StringComparison]::OrdinalIgnoreCase)
+		} | Sort-Object Name -Descending)
+	foreach ($old in ($stale | Select-Object -Skip $Keep)) {
+		Remove-Item -LiteralPath $old.FullName -Recurse -Force
+	}
+	return [PSCustomObject]@{ kept = [Math]::Min($stale.Count, $Keep); removed = [Math]::Max(0, $stale.Count - $Keep) }
+}
+
 function Start-Stand {
@@ (в Prepare-RunWorld, после Rename-Item)
 		Rename-Item -LiteralPath $script:activeWorldDir -NewName (Split-Path -Leaf $staleDir)
+		$cleanup = Remove-OldStaleWorlds (Split-Path -Parent $script:activeWorldDir) 2
+		Write-Host ("stale worlds: kept={0}; removed={1}" -f $cleanup.kept, $cleanup.removed)
 	}
```

## Тест (предлагаемый, тот же diff не применён)

Расширение `tools/testclient/tests/world-protection.Tests.ps1`:

```powershell
# После существующей ours-ротации: три старые папки -> остаются две новейшие.
foreach ($i in 1..3) {
	Set-Content -LiteralPath (Join-Path $root "world.stale-2026010$i-000000-000") -Value "s$i" -Force
}
$cleanup = Remove-OldStaleWorlds $root 2
$left = @(Get-ChildItem -LiteralPath $root -Directory -Filter 'world.stale-*')
if ($left.Count -ne 2) { throw "stale cleanup kept $($left.Count), want 2" }
if ($left.Name -notcontains 'world.stale-20260103-000000-000') { throw 'cleanup dropped the newest copy' }
try { Remove-OldStaleWorlds 'E:\Windows' 2; throw 'cleanup ran outside the data root' } catch { if ($_.Exception.Message -notmatch 'refusing to clean') { throw } }
```

Границы теста: пишет только в свой временный корень (`$root`), проверяет и число
оставшихся, и отказ вне `E:\MineRust-data`.

## Что решить владельцу

- Оставлять ровно **2** копии или другое число (сейчас предложено 2, как
  спросили).
- Учитывать ли размер (`~` сотни МБ на копию) — при желании можно добавить
  предел в гигабайтах отдельным правилом, но это уже другая задача.
