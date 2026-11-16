@echo off
setlocal
rem Launch two Fabric test clients (Alice and Bob by default) for a manual
rem two-player check against one server.
rem
rem Usage: run-two-clients.bat [address] [nick1] [nick2]
rem   address - server to auto-connect (default 127.0.0.1:25565)
rem   nick1   - first offline player name (default Alice)
rem   nick2   - second offline player name (default Bob)
rem
rem Each client gets its own gameDir under E:\MineRust-data\clients\<nick> and
rem its own testclient control port (25571 for the first, 25572 for the
rem second), so the two instances never share a cache or a control socket.
rem The modded client must be built first; see tools\testclient.
rem Closing a client's console window stops that client.

set "ADDR=127.0.0.1:25565"
if not "%~1"=="" set "ADDR=%~1"
set "NICK1=Alice"
set "NICK2=Bob"
if not "%~2"=="" set "NICK1=%~2"
if not "%~3"=="" set "NICK2=%~3"

if not exist "%~dp0testclient\launch-fabric-client.ps1" (
    echo launcher not found: %~dp0testclient\launch-fabric-client.ps1
    exit /b 2
)

echo Starting two test clients against %ADDR% ...
echo   %NICK1%  gameDir E:\MineRust-data\clients\%NICK1%  control 25571
echo   %NICK2%  gameDir E:\MineRust-data\clients\%NICK2%  control 25572
echo.

start "MineRust %NICK1%" cmd /k powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0testclient\launch-fabric-client.ps1" -Nick "%NICK1%" -Address "%ADDR%" -ControlPort 25571
start "MineRust %NICK2%" cmd /k powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0testclient\launch-fabric-client.ps1" -Nick "%NICK2%" -Address "%ADDR%" -ControlPort 25572

echo Two launcher windows opened. Logs:
echo   E:\MineRust-data\clients\%NICK1%\logs\latest.log
echo   E:\MineRust-data\clients\%NICK2%\logs\latest.log
endlocal