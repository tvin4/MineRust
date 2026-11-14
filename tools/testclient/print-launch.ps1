# Prints the composed launch command for a client without starting it.
# Debug helper for client.bat / launch-client.ps1.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Nick,
    [string]$Address = '127.0.0.1:25565'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$launch = & (Join-Path $here 'New-ClientLaunch.ps1') -Nick $Nick -Address $Address

Write-Host 'java exe   :' $launch.JavaExe
Write-Host 'main class :' $launch.MainClass
Write-Host 'gameDir    :' $launch.GameDir
Write-Host 'natives    :' $launch.NativesDir
Write-Host 'uuid       :' $launch.Uuid
Write-Host '--- jvm args ---'
$launch.JvmArgs | ForEach-Object { Write-Host $_ }
Write-Host '--- game args ---'
$launch.GameArgs | ForEach-Object { Write-Host $_ }
