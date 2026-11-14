@echo off
setlocal
rem Start the official vanilla server for hand comparisons (port 25566).
rem This script only launches the jar that is already there: it does not
rem create, copy or modify a world.

set "VANILLA_DIR=E:\MineRust-data\vanilla"
set "VANILLA_JAR=%VANILLA_DIR%\server.jar"

cd /d "%VANILLA_DIR%" 2>nul
if errorlevel 1 (
    echo.
    echo The vanilla server directory was not found:
    echo   %VANILLA_DIR%
    echo.
    echo Put it there with its eula.txt and server.properties
    echo (server-port=25566) plus an official server.jar.
    echo.
    pause
    exit /b 1
)

if not exist "%VANILLA_JAR%" set "VANILLA_JAR=E:\MineRust-data\server.jar"

if not exist "%VANILLA_JAR%" (
    echo.
    echo No server.jar was found. Looked in:
    echo   %VANILLA_DIR%\server.jar
    echo   E:\MineRust-data\server.jar
    echo.
    echo Download the official server.jar from Mojang's version manifest
    echo and put it in one of those places.
    echo.
    pause
    exit /b 1
)

echo Using jar: %VANILLA_JAR%
echo Starting the vanilla server (working directory %CD%) ...
java -jar "%VANILLA_JAR%" nogui
echo.
echo The vanilla server has stopped.
pause
