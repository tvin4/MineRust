@echo off
setlocal
rem Block persistence check (step 21): break blocks, stop the server,
rem restart with the same save_dir and check the same coordinates.
rem The save_dir itself is configured in server.toml (point 1).
rem ASCII only: keep this file readable under any console code page.
rem
rem Optional: pass another config path as the first argument.

cd /d "%~dp0.."

set "CONFIG=%~1"
if "%CONFIG%"=="" set "CONFIG=server.toml"
if not exist "%CONFIG%" (
    echo.
    echo Config not found: %CONFIG%
    echo Run tools\run-server.bat once; it explains the expected fields.
    echo.
    pause
    exit /b 1
)

set "SAVE="
for /f "tokens=2 delims='" %%a in ('findstr /b /c:"save_dir" "%CONFIG%"') do if not defined SAVE set "SAVE=%%a"
if not defined SAVE set "SAVE=(see save_dir in %CONFIG%)"

echo ============================================================
echo  MineRust block persistence check (step 21)
echo ============================================================
echo  Config:   %CONFIG%
echo  save_dir: %SAVE%
echo.
echo  1. Join, press F3, break 3 blocks and note the coordinates.
echo  2. Quit the client, then stop this server with Ctrl+C.
echo  3. Start this script again and revisit the coordinates.
echo.
echo  Expected until world saving (21a) is implemented:
echo    the blocks are back (the world is reloaded from the template).
echo  After 21a: the blocks stay broken and save_dir gets chunks\.
echo.
echo ============================================================
echo.

cargo run -p server --release -- --config "%CONFIG%"
echo.
echo The MineRust server has stopped.
pause