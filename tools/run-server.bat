@echo off
setlocal
rem Start the MineRust server from the repository root.
rem ASCII only: keep this file readable under any console code page.

cd /d "%~dp0.."

if not exist "server.toml" (
    echo.
    echo server.toml was not found in:
    echo   %CD%
    echo.
    echo Copy server.example.toml to server.toml and fill in the paths from
    echo YOUR copy of the game. Example for a C:\MineRust-data install:
    echo.
    echo   listen = "0.0.0.0:25565"
    echo   client_jar = "C:\MineRust-data\client.jar"
    echo   packets_report = "C:\MineRust-data\reports\packets.json"
    echo   registries_report = "C:\MineRust-data\reports\registries.json"
    echo   blocks_report = "C:\MineRust-data\reports\blocks.json"
    echo   world_dir = "C:\MineRust-data\world-template"
    echo   vanilla_server_jar = "C:\MineRust-data\server.jar"
    echo   vanilla_server_dir = "C:\MineRust-data\vanilla"
    echo.
    echo The reports are produced from the client jar by
    echo tools\generate-reports.ps1. See server.example.toml for every field.
    echo.
    pause
    exit /b 1
)

echo Starting the MineRust server (release build) ...
cargo run -p server --release
echo.
echo The MineRust server has stopped.
pause
