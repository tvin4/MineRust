@echo off
setlocal
rem Start the MineRust server and keep a copy of its console in logs\.
rem
rem Use this for live tests: the console of run-server.bat is lost when the
rem window closes, which is why the 16.11.2026 client error has no server log.
rem ASCII only: keep this file readable under any console code page.
rem
rem Optional: set RUST_LOG before calling to change the level. The default is
rem net=info,server=info (joins, chunk batches, kicks, timeouts); net=debug
rem also logs every packet and grows the file fast.

cd /d "%~dp0.."
if not exist "server.toml" (
    echo server.toml was not found in:
    echo   %CD%
    echo Copy server.example.toml to server.toml first; run-server.bat prints
    echo the expected fields when the file is missing.
    exit /b 1
)

set "LEVEL=%RUST_LOG%"
if "%LEVEL%"=="" set "LEVEL=net=info,server=info"

powershell -NoProfile -ExecutionPolicy Bypass -Command "[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false); $env:RUST_LOG='%LEVEL%'; $log = Join-Path (Get-Location) ('logs\server-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log'); New-Item -ItemType Directory -Force (Split-Path $log) | Out-Null; Write-Host ('server log: ' + $log); $utf8 = New-Object System.Text.UTF8Encoding($false); $writer = New-Object System.IO.StreamWriter($log, $false, $utf8); $writer.AutoFlush = $true; try { & cmd /c 'cargo run -p server --release 2>&1' | ForEach-Object { $line = [string]$_; $writer.WriteLine($line); $line } } finally { $writer.Dispose() }"

echo.
echo The MineRust server has stopped. The log is kept in logs\.
pause