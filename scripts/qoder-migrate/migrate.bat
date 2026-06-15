@echo off
rem ============================================================
rem  Qoder 跨机迁移助手 - 双击入口(BAT)
rem  作者:德姨
rem  用法:双击运行,菜单选 1/2/3/4
rem ============================================================
setlocal EnableExtensions EnableDelayedExpansion
chcp 65001 >nul

rem 切到脚本自身所在目录
pushd "%~dp0"

rem 标题
title Qoder Migrate - Launcher

:MENU
cls
echo ============================================================
echo            Qoder 跨机迁移助手 - 启动器
echo ============================================================
echo.
echo   主人您要做哪件事?
echo.
echo   [1] 提取 (A 机)  - 从本机 Qoder 导出 refresh_token
echo   [2] 注入 (B 机)  - 把 JSON 注入到本机 Qoder state.vscdb
echo   [3] 生成/重置 配置文件
echo   [4] 显示 当前路径扫描 (看看 state.vscdb 在哪)
echo   [0] 退出
echo.
echo   提示:
echo     - extract 之前请先关闭 Qoder IDE(避免 SQLite 锁)
echo     - inject 之前请先关闭 B 机 Qoder IDE
echo     - 配置文件是同目录的 qoder_migrate.config.json
echo.

set "CHOICE="
set /p CHOICE=主人请选 (0/1/2/3/4): 

if "%CHOICE%"=="0" goto :QUIT
if "%CHOICE%"=="1" goto :EXTRACT
if "%CHOICE%"=="2" goto :INJECT
if "%CHOICE%"=="3" goto :INIT
if "%CHOICE%"=="4" goto :SHOW
echo.
echo   [警告] 不认得您的选择,请重试。
timeout /t 2 >nul
goto :MENU

:EXTRACT
echo.
echo === 提取模式 (A 机) ===
echo.
where python >nul 2>&1
if errorlevel 1 (
    echo [错误] 没找到 python,请先装 Python 3.10+。
    echo        下载:https://www.python.org/downloads/windows/
    pause
    goto :MENU
)
python qoder_migrate.py extract -c qoder_migrate.config.json
echo.
echo [完成] 退出码 = %ERRORLEVEL%
echo.
pause
goto :MENU

:INJECT
echo.
echo === 注入模式 (B 机) ===
echo.
where python >nul 2>&1
if errorlevel 1 (
    echo [错误] 没找到 python,请先装 Python 3.10+。
    pause
    goto :MENU
)
python qoder_migrate.py inject -c qoder_migrate.config.json
echo.
echo [完成] 退出码 = %ERRORLEVEL%
echo.
pause
goto :MENU

:INIT
echo.
echo === 生成默认配置 ===
echo.
where python >nul 2>&1
if errorlevel 1 (
    echo [错误] 没找到 python,请先装 Python 3.10+。
    pause
    goto :MENU
)
if exist qoder_migrate.config.json (
    echo [警告] 配置文件已存在,会被覆盖。继续吗?
    set "OK="
    set /p OK="(Y/N): "
    if /i not "%OK%"=="Y" goto :MENU
)
python qoder_migrate.py init -c qoder_migrate.config.json
echo.
echo [完成] 退出码 = %ERRORLEVEL%
echo.
pause
goto :MENU

:SHOW
echo.
echo === 路径扫描 ===
echo.
where python >nul 2>&1
if errorlevel 1 (
    echo [错误] 没找到 python,请先装 Python 3.10+。
    pause
    goto :MENU
)
python qoder_migrate.py show -c qoder_migrate.config.json
echo.
echo [完成] 退出码 = %ERRORLEVEL%
echo.
pause
goto :MENU

:QUIT
popd
endlocal
exit /b 0
