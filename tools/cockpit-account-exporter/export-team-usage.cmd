@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Export-CockpitAccountData.ps1" %*
exit /b %ERRORLEVEL%
