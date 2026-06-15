@echo off
rem ============================================================
rem  Qoder 迁移助手 - 环境检查 + 依赖安装 BAT
rem  作者:德姨
rem  用法:双击运行,主人在另一台机器上测环境时先跑这一只
rem
rem  检查项:
rem    [1] Python 3.10+ 是否安装
rem    [2] pip 是否可用
rem    [3] cryptography 库是否已装(没有就装)
rem
rem  退出码:
rem    0 = 一切就绪
rem    1 = Python 没装或版本太低(致命)
rem    2 = pip 不可用(致命)
rem    3 = cryptography 装失败(可降级)
rem ============================================================
setlocal EnableExtensions
chcp 65001 >nul

title Qoder Migrate - Environment Check

set "EXIT_CODE=0"
set "PY_OK=0"
set "PIP_OK=0"
set "CRYPTO_OK=0"
set "PY_VERSION="
set "PYTHON_CMD="

echo.
echo ============================================================
echo        Qoder 跨机迁移助手 - 环境检查 (BAT)
echo ============================================================
echo.

rem ── 1. 检查 Python ─────────────────────────────
echo [1/3] 检查 Python 3.10+ ...
echo.

where python  >nul 2>&1
if not errorlevel 1 (
    set "PYTHON_CMD=python"
    goto :PY_FOUND
)
where python3 >nul 2>&1
if not errorlevel 1 (
    set "PYTHON_CMD=python3"
    goto :PY_FOUND
)
where py >nul 2>&1
if not errorlevel 1 (
    set "PYTHON_CMD=py -3"
    goto :PY_FOUND
)

:PY_NOT_FOUND
echo        [X]  未找到 python / python3 / py
echo             请先装 Python 3.10 或更高版本:
echo             https://www.python.org/downloads/windows/
echo.
echo             安装时务必勾选 "Add Python to PATH"
echo.
set "EXIT_CODE=1"
goto :SUMMARY

:PY_FOUND
echo        [OK] 找到 Python 启动器: %PYTHON_CMD%

rem 先验证 python 真的能跑(防 --version 输出"Python was unexpected"这种)
%PYTHON_CMD% -c "import sys" 1>nul 2>nul
if errorlevel 1 goto :PY_BROKEN

rem 用 python 自己解析版本号,比 --version 输出可靠得多
for /f "tokens=*" %%V in ('%PYTHON_CMD% -c "import sys; print(sys.version)"') do set "PY_VERSION=%%V"
echo        [OK] Python 版本: %PY_VERSION%

rem 用 sys.version_info 拿主次版本号,Python 自己解析,免去德姨自己 split
for /f "tokens=1,2" %%a in ('%PYTHON_CMD% -c "import sys; v=sys.version_info; print(v.major, v.minor)"') do (
    set "MAJOR=%%a"
    set "MINOR=%%b"
)
if "%MAJOR%"=="" set "MAJOR=0"
if "%MINOR%"=="" set "MINOR=0"

if %MAJOR% LSS 3 goto :PY_TOO_OLD
if %MAJOR% EQU 3 if %MINOR% LSS 10 goto :PY_TOO_OLD

echo        [OK] 版本满足 3.10+ 要求
set "PY_OK=1"
echo.
goto :PY_DONE

:PY_BROKEN
echo        [X]  Python 启动器 %PYTHON_CMD% 存在但无法执行!
echo             可能原因:
echo               1) Python 安装损坏,建议卸载重装 3.10+
echo               2) PATH 里有多个 Python 互相冲突
echo               3) %PYTHON_CMD% 是某个同名但非 Python 的程序
echo             排查:在 cmd 里手敲:
echo               %PYTHON_CMD% -c "print(1)"
echo.
set "EXIT_CODE=1"
goto :SUMMARY

:PY_TOO_OLD
echo        [X]  Python %MAJOR%.%MINOR% 低于 3.10,需要 3.10+
set "EXIT_CODE=1"
goto :SUMMARY

:PY_DONE

rem ── 2. 检查 pip ─────────────────────────────
echo [2/3] 检查 pip ...
echo.
%PYTHON_CMD% -m pip --version >nul 2>&1
if errorlevel 1 goto :PIP_MISSING

for /f "tokens=*" %%V in ('%PYTHON_CMD% -m pip --version 2^>^&1') do (
    echo        [OK] %%V
)
set "PIP_OK=1"
echo.
goto :PIP_DONE

:PIP_MISSING
echo        [X]  pip 不可用,请修复 Python 安装
echo             修复方法:重新运行 Python 安装包,选 "Modify" - 勾 pip
echo.
set "EXIT_CODE=2"
goto :SUMMARY

:PIP_DONE

rem ── 3. 检查 cryptography ─────────────────────────────
echo [3/3] 检查 cryptography (Qoder 解密必需) ...
echo.
%PYTHON_CMD% -c "import cryptography; print('        [OK] cryptography', cryptography.__version__, '已安装')" 2>nul
if not errorlevel 1 goto :CRYPTO_OK

echo        [-]  cryptography 未安装,正在尝试安装...
echo.
%PYTHON_CMD% -m pip install --upgrade pip >nul 2>&1
%PYTHON_CMD% -m pip install cryptography
if errorlevel 1 goto :CRYPTO_FAIL

echo.
%PYTHON_CMD% -c "import cryptography; print('        [OK] cryptography', cryptography.__version__, '安装成功')" 2>nul
if not errorlevel 1 goto :CRYPTO_OK

:CRYPTO_FAIL
echo.
echo        [X]  cryptography 安装失败!
echo             常见原因:
echo               1) 公司网络需要代理: set HTTPS_PROXY=http://your-proxy:port
echo               2) Python 版本太旧(虽然上面已经检查过,极少数情况)
echo               3) pip 缓存损坏: %PYTHON_CMD% -m pip cache purge
echo.
set "EXIT_CODE=3"
goto :SUMMARY

:CRYPTO_OK
set "CRYPTO_OK=1"

:SUMMARY
echo.
echo ============================================================
echo                     检查结果汇总
echo ============================================================
echo.
if "%PY_OK%"=="1" (
    echo        [OK] Python %PY_VERSION%
) else (
    echo        [X ] Python 不可用
)
if "%PIP_OK%"=="1" (
    echo        [OK] pip
) else (
    echo        [X ] pip 不可用
)
if "%CRYPTO_OK%"=="1" (
    echo        [OK] cryptography
) else (
    echo        [X ] cryptography 未就绪
)
echo.
echo        退出码: %EXIT_CODE%
echo.

if "%EXIT_CODE%"=="0" (
    echo        主人,环境就绪。可以双击 migrate.bat 开始迁移。
) else if "%EXIT_CODE%"=="1" (
    echo        主人,Python 没装或版本太低。请先装 Python 3.10+ 后重跑。
) else if "%EXIT_CODE%"=="2" (
    echo        主人,pip 不可用。请修复 Python 安装。
) else if "%EXIT_CODE%"=="3" (
    echo        主人,cryptography 装失败。请按上面提示排查。
)
echo.
echo ============================================================

rem ── 退出前停顿(防双击窗口自动关闭)───────────────────────────
if "%EXIT_CODE%"=="0" (
    echo        按任意键关闭此窗口
) else (
    echo        [出错] 按任意键关闭此窗口(主人请把屏幕截给德姨看)
)
echo.
pause >nul

endlocal & exit /b %EXIT_CODE%
