@echo off
rem MineRust testclient, stage A: launches one vanilla 26.3 client.
rem Usage: client.bat <nick> [address]
rem   nick     - offline player name (its own gameDir under E:\MineRust-data\clients)
rem   address  - server to auto-connect (default 127.0.0.1:25565)
rem
rem Heap: -Xms512M -Xmx2048M (owner's rule). Token is a stub; the client is
rem for offline-mode servers on 127.0.0.1 only.
setlocal
if "%~1"=="" (
    echo Usage: client.bat ^<nick^> [address]
    exit /b 2
)
set "NICK=%~1"
set "ADDR=127.0.0.1:25565"
if not "%~2"=="" set "ADDR=%~2"

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0launch-client.ps1" -Nick "%NICK%" -Address "%ADDR%"
endlocal
