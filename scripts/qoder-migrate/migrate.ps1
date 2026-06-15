# ============================================================
#  Qoder 跨机迁移助手 - 启动器 (PowerShell 彩色版)
#  作者:德姨
#  用法:右键 migrate.ps1 -> "使用 PowerShell 运行"
#       (双击 .ps1 默认会被 ExecutionPolicy 拦,推荐双击 migrate.bat)
# ============================================================

$ErrorActionPreference = 'Stop'
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $ScriptDir

function Show-Banner {
    Clear-Host
    Write-Host '============================================================' -ForegroundColor Cyan
    Write-Host '            Qoder 跨机迁移助手 - 启动器 (PowerShell)' -ForegroundColor Yellow
    Write-Host '============================================================' -ForegroundColor Cyan
}

function Test-Python {
    try {
        $py = (Get-Command python -ErrorAction Stop).Source
        Write-Host ("[OK] Python 路径: {0}" -f $py) -ForegroundColor Green
        & python --version
        return $true
    } catch {
        Write-Host '[错误] 没找到 python。' -ForegroundColor Red
        Write-Host '       请装 Python 3.10+: https://www.python.org/downloads/windows/' -ForegroundColor Red
        return $false
    }
}

function Test-Cryptography {
    try {
        & python -c "import cryptography; print('[OK] cryptography:', cryptography.__version__)" | Out-Host
        if ($LASTEXITCODE -ne 0) { throw 'cryptography not installed' }
        return $true
    } catch {
        Write-Host '[警告] 缺少 cryptography 库。正在帮您装...' -ForegroundColor Yellow
        & python -m pip install --user cryptography
        if ($LASTEXITCODE -ne 0) {
            Write-Host '[错误] pip 装失败,请手动: pip install cryptography' -ForegroundColor Red
            return $false
        }
        return $true
    }
}

function Run-Step {
    param(
        [string]$Title,
        [string]$Cmd,
        [string]$Hint
    )
    Write-Host ''
    Write-Host ("=== {0} ===" -f $Title) -ForegroundColor Magenta
    if ($Hint) { Write-Host ("提示:{0}" -f $Hint) -ForegroundColor DarkGray }
    Write-Host ''
    Write-Host ("$ " + $Cmd) -ForegroundColor DarkCyan
    & powershell -Command $Cmd
    Write-Host ''
    Write-Host ("[完成] 退出码 = {0}" -f $LASTEXITCODE) -ForegroundColor Yellow
    Write-Host ''
    Read-Host '按 Enter 返回菜单'
}

# ─── 主菜单循环 ───
while ($true) {
    Show-Banner
    Write-Host '主人您要做哪件事?' -ForegroundColor White
    Write-Host ''
    Write-Host '  [1] 提取 (A 机)  - 从本机 Qoder 导出 refresh_token' -ForegroundColor Green
    Write-Host '  [2] 注入 (B 机)  - 把 JSON 注入到本机 Qoder state.vscdb' -ForegroundColor Green
    Write-Host '  [3] 生成/重置 配置文件' -ForegroundColor Green
    Write-Host '  [4] 显示 当前路径扫描' -ForegroundColor Green
    Write-Host '  [0] 退出' -ForegroundColor DarkGray
    Write-Host ''
    Write-Host '提示:' -ForegroundColor DarkYellow
    Write-Host '  - extract 之前请先关闭 Qoder IDE(避免 SQLite 锁)' -ForegroundColor DarkYellow
    Write-Host '  - inject 之前请先关闭 B 机 Qoder IDE' -ForegroundColor DarkYellow
    Write-Host '  - 配置文件是同目录的 qoder_migrate.config.json' -ForegroundColor DarkYellow
    Write-Host ''

    $choice = Read-Host '主人请选 (0/1/2/3/4)'
    switch ($choice) {
        '0' {
            Write-Host '德姨告退。' -ForegroundColor Cyan
            return
        }
        '1' {
            if (-not (Test-Python)) { Read-Host '按 Enter 返回'; continue }
            if (-not (Test-Cryptography)) { Read-Host '按 Enter 返回'; continue }
            Run-Step -Title '提取模式 (A 机)' `
                     -Cmd 'python qoder_migrate.py extract -c qoder_migrate.config.json' `
                     -Hint '请先关闭 Qoder IDE'
        }
        '2' {
            if (-not (Test-Python)) { Read-Host '按 Enter 返回'; continue }
            if (-not (Test-Cryptography)) { Read-Host '按 Enter 返回'; continue }
            Run-Step -Title '注入模式 (B 机)' `
                     -Cmd 'python qoder_migrate.py inject -c qoder_migrate.config.json' `
                     -Hint '请先关闭 B 机 Qoder IDE'
        }
        '3' {
            if (-not (Test-Python)) { Read-Host '按 Enter 返回'; continue }
            $cfgPath = Join-Path $ScriptDir 'qoder_migrate.config.json'
            if (Test-Path $cfgPath) {
                Write-Host "[警告] 配置文件已存在,会被覆盖。继续吗?(Y/N)" -ForegroundColor Yellow
                $ok = Read-Host '(Y/N)'
                if ($ok -ne 'Y') { continue }
            }
            Run-Step -Title '生成默认配置' `
                     -Cmd 'python qoder_migrate.py init -c qoder_migrate.config.json'
        }
        '4' {
            if (-not (Test-Python)) { Read-Host '按 Enter 返回'; continue }
            Run-Step -Title '路径扫描' `
                     -Cmd 'python qoder_migrate.py show -c qoder_migrate.config.json'
        }
        default {
            Write-Host '[警告] 不认得您的选择,请重试。' -ForegroundColor Yellow
            Start-Sleep -Seconds 1
        }
    }
}
