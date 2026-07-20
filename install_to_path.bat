@echo off
title StreamZip - Add to Windows PATH
echo ====================================================================
echo    StreamZip - Automatic Windows PATH Installer
echo ====================================================================
echo.

set "TARGET_DIR=%~dp0"
set "TARGET_DIR=%TARGET_DIR:~0,-1%"

if exist "%TARGET_DIR%\target\release\strzip.exe" (
    copy /y "%TARGET_DIR%\target\release\strzip.exe" "%TARGET_DIR%\strzip.exe" >nul
)

echo Registering StreamZip location in your Windows User PATH:
echo "%TARGET_DIR%"
echo.

powershell -NoProfile -ExecutionPolicy Bypass -Command "$p = [Environment]::GetEnvironmentVariable('Path', 'User'); $t = $env:TARGET_DIR; if ($p -split ';' -notcontains $t) { [Environment]::SetEnvironmentVariable('Path', $p + ';' + $t, 'User') }"

echo ====================================================================
echo   SUCCESS! StreamZip is ready in your Windows PATH!
echo ====================================================================
echo.
echo   IMPORTANT NOTES FOR USERS:
echo   ------------------------------------------------------------------
echo   1. Open a NEW Command Prompt or PowerShell window for PATH changes
echo      to take effect.
echo.
echo   2. DO NOT DELETE or rename this folder! Windows looks for
echo      'strzip.exe' right here when you run it from terminal.
echo.
echo   3. HOW TO RUN:
echo      Open a NEW Command Prompt or PowerShell window ANYWHERE and type:
echo.
echo          strzip "path\to\your\game.part1.rar" --verify-first --no-log
echo.
echo   ------------------------------------------------------------------
echo.
echo Press any key to exit this installer...
pause >nul
