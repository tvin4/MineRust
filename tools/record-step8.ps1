<#
.SYNOPSIS
    Records one vanilla trace scenario (S1..S4) for MineRust step 8.

.DESCRIPTION
    Drives the whole scenario run so the owner does not assemble it by hand:
      1. fresh copy of world template v3 into E:\MineRust-data\vanilla\world;
      2. server.properties check (survival, force-gamemode, compression off,
         online-mode false, max-tick-time -1, port 25566) - stops with a clear
         error if anything is off;
      3. vanilla server on 25566 + mcproxy on 25565, each scenario writes into
         its own folder E:\MineRust-data\traces\<scenario>;
      4. prints the exact actions of the scenario and waits for Enter;
      5. stop: save-all flush + stop the vanilla, stop the proxy, sha256 the
         world copy against the template and list changed .mca files;
      6. runs tools\trace-analyze\m45-step80-scenarios.py on the observer
         stream and saves the report to <out>\summary.txt.

    Renames the raw connection files to <scenario>-A.jsonl (first client) and
    <scenario>-B.jsonl (observer) in the scenario folder.

.PARAMETER Scenario
    S1 | S2 | S3 | S4 - which scenario to record this run.

.PARAMETER DryRun
    Checks paths, settings and prints the scenario instructions without
    starting anything and without touching the world.

.NOTES
    IMPORTANT: disable QuickEdit mode in the console window (Properties ->
    Options -> uncheck "QuickEdit Mode"). A click in the console pauses the
    whole process (java included) and corrupts the timing of the trace.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('S1', 'S2', 'S3', 'S4')]
    [string]$Scenario,

    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

# --- paths ------------------------------------------------------------------
$repo        = Split-Path -Parent $PSScriptRoot
$vanillaDir  = 'E:\MineRust-data\vanilla'
$worldDir    = Join-Path $vanillaDir 'world'
$templateDir = 'E:\MineRust-data\world-template-v3'
$templateManifest = 'E:\MineRust-data\world-template-v3.sha256.txt'
$serverJar   = 'E:\MineRust-data\server.jar'
$propsPath   = Join-Path $vanillaDir 'server.properties'
$tracesRoot  = 'E:\MineRust-data\traces'
$outDir      = Join-Path $tracesRoot $Scenario.ToLower()
$analyzePy   = Join-Path $repo 'tools\trace-analyze\m45-step80-scenarios.py'
$scenarioKey = $Scenario.ToLower()   # s1..s4

function Say([string]$msg) { Write-Host $msg }
function Step([string]$msg) { Write-Host ("== " + $msg) -ForegroundColor Cyan }
function Warn([string]$msg) { Write-Host ("!! " + $msg) -ForegroundColor Yellow }
function Fail([string]$msg) {
    Write-Host ("ERROR: " + $msg) -ForegroundColor Red
    exit 1
}

Say ''
Say ('MineRust step 8 trace recorder - scenario ' + $Scenario)
Say ('=' * 60)
Warn 'QuickEdit mode must be OFF in this console (Properties -> Options).'
Warn 'A stray click pauses java and corrupts the trace timing.'
Say ''

# --- 0. paths ---------------------------------------------------------------
Step 'Checking paths'
foreach ($p in @($templateDir, $serverJar, $propsPath, $analyzePy)) {
    if (-not (Test-Path $p)) { Fail ('missing: ' + $p) }
    Say ('ok: ' + $p)
}
$port = (Select-String -Path $propsPath -Pattern '^server-port=(\d+)' |
    ForEach-Object { $_.Matches[0].Groups[1].Value })
if ($port -ne '25566') {
    Fail ('server.properties has server-port=' + $port + ', expected 25566')
}
Say 'ok: server-port=25566'

# --- 1. server.properties check --------------------------------------------
Step 'Checking server.properties'
$required = @{
    'gamemode'                      = 'survival'
    'force-gamemode'                = 'true'
    'network-compression-threshold' = '-1'
    'online-mode'                   = 'false'
    'max-tick-time'                 = '-1'
}
$propsText = Get-Content $propsPath
$bad = @()
foreach ($key in $required.Keys) {
    $want = $required[$key]
    $line = $propsText | Select-String -Pattern ('^' + [regex]::Escape($key) + '=(.*)$')
    $have = if ($line) { $line.Matches[0].Groups[1].Value.Trim() } else { '<absent>' }
    if ($have -ne $want) {
        $bad += ('  ' + $key + '=' + $have + '   expected ' + $key + '=' + $want)
    } else {
        Say ('ok: ' + $key + '=' + $have)
    }
}
if ($bad.Count -gt 0) {
    Warn 'server.properties does not match the runbook:'
    $bad | ForEach-Object { Warn $_ }
    Warn 'fix the values by hand and re-run (this script does not edit them).'
    Fail 'server.properties check failed'
}

# --- 2. fresh world copy ----------------------------------------------------
if ($DryRun) {
    Step 'DryRun: world copy NOT made, nothing started'
} else {
    Step 'Preparing a fresh world copy (template v3)'
    if (Test-Path $worldDir) { Remove-Item -Recurse -Force $worldDir }
    Copy-Item -Recurse $templateDir $worldDir
    Say ('ok: ' + $templateDir + ' -> ' + $worldDir)
    if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }
    Say ('ok: trace folder ' + $outDir)
}

# --- 3. scenario instructions ----------------------------------------------
Step ('Scenario ' + $Scenario + ' - what to do')
Say 'Players: A = the acting client (connects FIRST), B = the observer.'
Say 'Connect both to 127.0.0.1:25565 (the proxy). Pause >= 3 s between actions.'
Say ''
switch ($Scenario) {
    'S1' {
        Say 'S1 (entering with a rotation):'
        Say '  1. Client A connects to the proxy.'
        Say '  2. A turns to a NON-zero yaw/pitch (about 30-40 degrees, not 0/90).'
        Say '  3. A stands still >= 3 s.'
        Say '  4. A disconnects, then connects AGAIN (the join must now carry'
        Say '     the saved rotation - check the join if it came back 0/0: leave,'
        Say '     turn again and rejoin).'
        Say '  5. A stands still >= 3 s.'
        Say '  6. Only now client B connects and looks at A.'
        Say '  Watch for: add_entity of A (angle bytes) and rotate_head in B stream.'
    }
    'S2' {
        Say 'S2 (sneak pose):'
        Say '  1. A connects first, then B connects and points the camera at A.'
        Say '  2. A holds Shift (crouch) >= 3 s, releases, waits >= 3 s.'
        Say '  3. Repeat the press/release 2-3 times.'
        Say '  Watch for: set_entity_data (pose index/type/value) in B stream.'
    }
    'S3' {
        Say 'S3 (swings + block break):'
        Say '  1. A connects first, then B connects and looks at A.'
        Say '  2. A swings the arm (left click in the air) THREE times,'
        Say '     >= 3 s pause between swings.'
        Say '  3. A breaks ONE block, then stands >= 3 s.'
        Say '  Watch for: animate and entity_event in B stream.'
    }
    'S4' {
        Say 'S4 (sprint + swimming):'
        Say '  1. A connects first, then B connects and looks at A.'
        Say '  2. A sprints on land >= 3 s.'
        Say '  3. Run:  /tp A 106 41 -31   (A is teleported into the pond).'
        Say '  4. A swims with sprint held >= 3 s.'
        Say '  Watch for: animate, set_entity_data (swim pose), update_attributes'
        Say '  in B stream.'
    }
}
Say ''
Say 'When done, come back to THIS window and press Enter.'

if ($DryRun) {
    Step 'DryRun: stopping here (nothing started, world untouched)'
    Say ''
    Say ('dry run OK - settings and paths are ready for scenario ' + $Scenario)
    exit 0
}

Read-Host 'Press Enter to start the recording (clients can connect once it is up)'

# --- 4. start vanilla + proxy ----------------------------------------------
Step 'Starting vanilla server on 25566'
$java = New-Object System.Diagnostics.Process
$java.StartInfo.FileName = 'java'
$java.StartInfo.Arguments = '-Xmx1G -jar ' + $serverJar + ' --nogui'
$java.StartInfo.WorkingDirectory = $vanillaDir
$java.StartInfo.UseShellExecute = $false
$java.StartInfo.RedirectStandardInput = $true
$java.StartInfo.RedirectStandardOutput = $true
$java.StartInfo.RedirectStandardError = $true
if (-not $java.Start()) { Fail 'could not start java' }
$javaPid = $java.Id
Say ('ok: java pid ' + $javaPid)

$ready = $false
$deadline = (Get-Date).AddSeconds(120)
$logPath = Join-Path $vanillaDir 'logs\latest.log'
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 500
    if (Test-Path $logPath) {
        $tail = Get-Content $logPath -Tail 200 -ErrorAction SilentlyContinue
        if ($tail -match 'Done \(') { $ready = $true; break }
    }
    if ($java.HasExited) { Fail ('java exited during startup (code ' + $java.ExitCode + ')') }
}
if (-not $ready) { Fail 'vanilla did not report "Done (" within 120 s' }
Say 'ok: vanilla is up'

Step 'Starting mcproxy on 25565 (writing into ' + $outDir + ')'
$proxy = New-Object System.Diagnostics.Process
$proxy.StartInfo.FileName = 'cargo'
$proxy.StartInfo.Arguments = 'run -p parity --bin mcproxy -- --listen 127.0.0.1:25565 --out ' + $outDir
$proxy.StartInfo.WorkingDirectory = $repo
$proxy.StartInfo.UseShellExecute = $false
$proxy.StartInfo.RedirectStandardOutput = $true
$proxy.StartInfo.RedirectStandardError = $true
if (-not $proxy.Start()) { Fail 'could not start mcproxy' }
$proxyPid = $proxy.Id
Say ('ok: mcproxy pid ' + $proxyPid)
Start-Sleep -Seconds 2
if ($proxy.HasExited) { Fail 'mcproxy exited at once (compression on or online-mode true?)' }

Say ''
Say 'READY. Clients connect to 127.0.0.1:25565 now.'
Say 'Run the scenario steps above, then come back and press Enter.'

# --- 5. stop ----------------------------------------------------------------
Read-Host 'Press Enter to stop the recording'

Step 'Stopping vanilla (save-all flush, then stop)'
$java.StandardInput.WriteLine('save-all flush')
Start-Sleep -Seconds 2
$java.StandardInput.WriteLine('stop')
$java.StandardInput.Close()
$exited = $java.WaitForExit(30000)
if (-not $exited) {
    Warn 'vanilla did not stop in 30 s, killing it'
    $java.Kill()
} else {
    Say ('ok: vanilla stopped (exit code ' + $java.ExitCode + ')')
}

Step 'Stopping mcproxy'
if (-not $proxy.HasExited) {
    $proxy.Kill()
    $proxy.WaitForExit(5000) | Out-Null
}
Say 'ok: mcproxy stopped'

# --- 6. rename trace files --------------------------------------------------
Step 'Renaming trace files'
$rawA = Join-Path $outDir 'proxy-conn-1.jsonl'
$rawB = Join-Path $outDir 'proxy-conn-2.jsonl'
if (-not (Test-Path $rawA)) { Fail ('missing ' + $rawA + ' - did client A connect through the proxy?') }
if (-not (Test-Path $rawB)) { Fail ('missing ' + $rawB + ' - did observer B connect through the proxy?') }
$dstA = Join-Path $outDir ($scenarioKey + '-A.jsonl')
$dstB = Join-Path $outDir ($scenarioKey + '-B.jsonl')
Move-Item -Force $rawA $dstA
Move-Item -Force $rawB $dstB
Say ('ok: ' + $dstA)
Say ('ok: ' + $dstB)

# --- 7. sha256 of the world copy vs the template ---------------------------
Step 'Checking the world copy against the template'
function Get-Manifest([string]$root) {
    Get-ChildItem -Recurse -File $root | ForEach-Object {
        $rel = $_.FullName.Substring($root.Length).TrimStart('\')
        $h = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
        ($h + '  ' + $rel)
    } | Sort-Object { ($_ -split '  ', 2)[1] }
}
$haveManifest = Get-Manifest $worldDir
$havePath = Join-Path $outDir 'world.sha256.txt'
$haveManifest | Set-Content $havePath -Encoding ascii
Say ('ok: wrote ' + $havePath)

if (-not (Test-Path $templateManifest)) {
    Warn ('template manifest missing, creating ' + $templateManifest)
    Get-Manifest $templateDir | Set-Content $templateManifest -Encoding ascii
}
$wantManifest = Get-Content $templateManifest

$changed = @()
$wantMap = @{}
foreach ($line in $wantManifest) {
    $parts = $line -split '  ', 2
    if ($parts.Count -eq 2) { $wantMap[$parts[1].Trim()] = $parts[0].Trim() }
}
foreach ($line in $haveManifest) {
    $parts = $line -split '  ', 2
    if ($parts.Count -ne 2) { continue }
    $rel = $parts[1].Trim()
    $hash = $parts[0].Trim()
    if (-not $wantMap.ContainsKey($rel)) {
        $changed += ('added    ' + $rel)
    } elseif ($wantMap[$rel] -ne $hash) {
        $changed += ('modified ' + $rel)
    }
}
$mca = $changed | Where-Object { $_ -like '*\region\*.mca' }
Say ''
Say 'Changed files vs template (world copy only, template untouched):'
if ($changed.Count -eq 0) {
    Say '  (none)'
} else {
    $changed | ForEach-Object { Say ('  ' + $_) }
}
Say ''
Say ('Changed .mca region files: ' + $(if ($mca) { $mca.Count } else { 0 }))

# --- 8. analyze -------------------------------------------------------------
Step 'Analyzing the observer stream'
$summaryPath = Join-Path $outDir 'summary.txt'
$analyzeCmd = 'python ' + $analyzePy + ' --dir ' + $outDir + ' --scenario ' + $scenarioKey
Say ('run: ' + $analyzeCmd)
cmd /c ($analyzeCmd + ' > "' + $summaryPath + '" 2>&1')
if ($LASTEXITCODE -ne 0) {
    Warn 'analysis script exited non-zero, see summary.txt'
}
if (Test-Path $summaryPath) {
    Say ('ok: report saved to ' + $summaryPath)
} else {
    Warn ('no summary.txt produced at ' + $summaryPath)
}

Say ''
Say ('DONE. Scenario ' + $Scenario + ' recorded into ' + $outDir)
Say 'Next: copy the summary and the trace folder to the report, then run the next scenario.'
