@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Open-CockpitAccountExporter.ps1" %*
exit /b %ERRORLEVEL%
