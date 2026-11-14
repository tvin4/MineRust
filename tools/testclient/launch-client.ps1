# Launches one vanilla 26.3 test client (MineRust testclient, stage A).
# Thin wrapper over New-ClientLaunch.ps1: builds the command from the
# official version manifest, seeds options.txt, starts java, logs what it
# did. client.bat calls this.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Nick,
    [string]$Address = '127.0.0.1:25565',
    [int]$Width = 1280,
    [int]$Height = 720,
    [switch]$Wait
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$launch = & (Join-Path $here 'New-ClientLaunch.ps1') -Nick $Nick -Address $Address -Width $Width -Height $Height

# --- options.txt: render + simulation distance about 10 --------------------
# Key names are taken from the real options.txt of the 26.3 game
# (E:\instances\26.3Vanilla\minecraft\options.txt, verified 14.11.2026).
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
# The manifest's JVM arguments already carry -Djava.library.path and -cp
# (placeholders expanded by New-ClientLaunch.ps1); here we only append the
# main class and the game arguments. Heap limits -Xms512M -Xmx2048M are the
# owner's rule and are prepended by the builder.
$argList = @()
$argList += $launch.JvmArgs
$argList += $launch.MainClass
$argList += $launch.GameArgs

Write-Host ("client '{0}' -> {1} (offline uuid {2})" -f $Nick, $Address, $launch.Uuid)
Write-Host ("  java    : {0}" -f $launch.JavaExe)
Write-Host ("  gameDir : {0}" -f $launch.GameDir)
Write-Host ("  natives : {0}" -f $launch.NativesDir)
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
