# Launches one Fabric-modded 26.3 test client (MineRust testclient, stage B).
# Thin wrapper over New-FabricLaunch.ps1: same handling as launch-client.ps1
# (options.txt seeding, java start, logs) but with the Fabric loader profile
# and the testclient mod. client.bat calls launch-client.ps1 by default;
# pass -Fabric to this script for the modded client.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Nick,
    [string]$Address = '127.0.0.1:25565',
    [int]$Width = 1280,
    [int]$Height = 720,
    [int]$ControlPort = 25571,
    [switch]$Wait
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$launch = & (Join-Path $here 'New-FabricLaunch.ps1') -Nick $Nick -Address $Address `
    -Width $Width -Height $Height -ControlPort $ControlPort

# --- options.txt: render + simulation distance about 10 --------------------
# Same keys as launch-client.ps1 (real options.txt of the 26.3 game).
$opt = Join-Path $launch.GameDir 'options.txt'
if (Test-Path $opt) {
    $txt = Get-Content $opt -Raw -Encoding utf8
    $txt = $txt -replace 'renderDistance:\d+', 'renderDistance:10'
    $txt = $txt -replace 'simulationDistance:\d+', 'simulationDistance:10'
    [System.IO.File]::WriteAllText($opt, $txt)
} else {
    $lines = @(
        'version:5023'
        'renderDistance:10'
        'simulationDistance:10'
        'fullscreen:false'
        'maxFps:120'
        'lang:en_us'
    )
    [System.IO.File]::WriteAllLines($opt, $lines)
}

# --- java invocation -------------------------------------------------------
# Same structure as launch-client.ps1: JvmArgs carry -Djava.library.path
# and -cp (Fabric libs prepended by New-FabricLaunch.ps1), then main class
# (KnotClient) and the game arguments. Heap -Xms512M -Xmx2048M is the
# owner's rule, prepended by the builder.
# Arguments containing spaces (the loader profile's -DFabricMcEmu= ... are
# quoted, otherwise Start-Process splits them into separate arguments.
function Quote-Arg([string]$a) {
    if ($a -match '\s') { return '"' + $a + '"' }
    return $a
}
$argList = @()
$argList += $launch.JvmArgs | ForEach-Object { Quote-Arg $_ }
$argList += Quote-Arg $launch.MainClass
$argList += $launch.GameArgs | ForEach-Object { Quote-Arg $_ }

Write-Host ("fabric client '{0}' -> {1} (offline uuid {2})" -f $Nick, $Address, $launch.Uuid)
Write-Host ("  java    : {0}" -f $launch.JavaExe)
Write-Host ("  gameDir : {0}" -f $launch.GameDir)
Write-Host ("  natives : {0}" -f $launch.NativesDir)
Write-Host ("  control : 127.0.0.1:{0}" -f $launch.ControlPort)
Write-Host ("  memory  : -Xms512M -Xmx2048M")

$stdout = Join-Path $launch.GameDir 'client-stdout.log'
$stderr = Join-Path $launch.GameDir 'client-stderr.log'
$p = Start-Process -FilePath $launch.JavaExe -ArgumentList $argList -PassThru -NoNewWindow `
    -WorkingDirectory $launch.GameDir `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr
Write-Host ("  pid     : {0}" -f $p.Id)
Write-Host ("  logs    : {0} / {1}" -f $stdout, $stderr)
if ($Wait) {
    $p.WaitForExit()
    Write-Host ("exited with code {0}" -f $p.ExitCode)
}
return $p.Id
