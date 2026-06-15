<#
.SYNOPSIS
    Qoder 跨机迁移助手 — 提取 / 注入已登录 Qoder 账号 token。

.DESCRIPTION
    在 A 机执行 -Mode extract,导出 cockpit-tools 兼容 JSON(含 access_token / refresh_token);
    在 B 机执行 -Mode inject,把 JSON 注入到 Qoder 的 state.vscdb,使 Qoder 启动后即认为已登录。

    加密层(完全照搬 cockpit-tools 源码,见 src-tauri/src/modules/vscode_inject.rs:153-263):
        Local State 里的 os_crypt.encrypted_key (base64, 前缀 "DPAPI")
        -> CryptUnprotectData -> 32 字节 AES-256 key
        -> AES-256-GCM (v10 前缀 + 12 字节 nonce + ciphertext + 16 字节 tag)

    SQLite 读法:
        PowerShell 7+ 用 Microsoft.Data.Sqlite(内置);
        PowerShell 5.1 fallback 用 System.Data.SQLite(.NET Framework 内置)。

    零外部依赖:不引 NuGet,不下 sqlite3.exe,不调 Python。

.PARAMETER Mode
    extract | inject | show

.PARAMETER InputJson
    inject 模式: 要注入的 JSON 文件路径(来自 A 机 extract 的产物)。

.PARAMETER OutputJson
    extract 模式: 输出 JSON 路径,默认 .\qoder_credentials.json。

.PARAMETER QoderUserDataDir
    可选,覆盖默认 %APPDATA%\Qoder。

.EXAMPLE
    # A 机
    pwsh -ExecutionPolicy Bypass -File .\QoderMigrate.ps1 -Mode extract -OutputJson .\qoder_credentials.json

    # 复制到 B 机后
    pwsh -ExecutionPolicy Bypass -File .\QoderMigrate.ps1 -Mode inject -InputJson .\qoder_credentials.json

.NOTES
    Author  : 德姨
    Tested  : Windows 11 23H2 + PowerShell 7.4
    Requires: PowerShell 7+ (因为 .NET 6+ 的 AesGcm 是 GCM 唯一原生路径;5.1 没 GCM)
    Source  : 复刻自 cockpit-tools src-tauri/src/modules/{vscode_inject,qoder_account,qoder_instance}.rs
#>

[CmdletBinding()]
param(
    [ValidateSet('extract', 'inject', 'show')]
    [string]$Mode = 'show',

    [string]$InputJson,
    [string]$OutputJson = (Join-Path $PWD 'qoder_credentials.json'),
    [string]$QoderUserDataDir
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# ============== 常量(对照源码) ==============
$SECRET_USER_INFO  = 'secret://aicoding.auth.userInfo'
$SECRET_USER_PLAN  = 'secret://aicoding.auth.userPlan'
$SECRET_CREDIT     = 'secret://aicoding.auth.creditUsage'
$V10_PREFIX        = [byte[]]([byte]'v', [byte]'1', [byte]'0')

# ============== 路径解析 ==============
function Get-QoderPaths {
    param([string]$Override)

    if ($Override) {
        $root = (Resolve-Path -LiteralPath $Override).Path
    } elseif ($env:APPDATA) {
        $root = Join-Path $env:APPDATA 'Qoder'
    } else {
        throw '未设置 APPDATA,也无法解析 Qoder 用户数据目录'
    }

    [pscustomobject]@{
        Root         = $root
        LocalState   = Join-Path $root 'Local State'
        StateDb      = Join-Path $root 'User\globalStorage\state.vscdb'
        MachineToken = Join-Path $root 'SharedClientCache\cache\machine_token.json'
        MachineId    = Join-Path $root 'SharedClientCache\cache\id'
    }
}

# ============== DPAPI P/Invoke ==============
function Invoke-DpapiUnprotect {
    param([byte[]]$Cipher)

    if (-not ('QoderMig.Native' -as [type])) {
        Add-Type -Namespace QoderMig -Name Native -MemberDefinition @'
            [System.Runtime.InteropServices.StructLayout(
                System.Runtime.InteropServices.LayoutKind.Sequential)]
            public struct DATA_BLOB {
                public int cbData;
                public System.IntPtr pbData;
            }

            [System.Runtime.InteropServices.DllImport("crypt32.dll",
                SetLastError = true,
                CallingConvention = System.Runtime.InteropServices.CallingConvention.StdCall)]
            public static extern bool CryptUnprotectData(
                ref DATA_BLOB pDataIn,
                System.IntPtr ppszDataDescr,
                System.IntPtr pOptionalEntropy,
                System.IntPtr pvReserved,
                System.IntPtr pPromptStruct,
                int dwFlags,
                ref DATA_BLOB pDataOut);

            [System.Runtime.InteropServices.DllImport("kernel32.dll",
                SetLastError = true)]
            public static extern System.IntPtr LocalFree(System.IntPtr hMem);
'@
    }

    $inBlob  = New-Object QoderMig.Native+DATA_BLOB
    $outBlob = New-Object QoderMig.Native+DATA_BLOB
    $inBlob.cbData = $Cipher.Length
    $inBlob.pbData = [System.Runtime.InteropServices.Marshal]::AllocHGlobal($Cipher.Length)
    try {
        [System.Runtime.InteropServices.Marshal]::Copy($Cipher, 0, $inBlob.pbData, $Cipher.Length)
        $ok = [QoderMig.Native]::CryptUnprotectData(
            [ref]$inBlob, [System.IntPtr]::Zero, [System.IntPtr]::Zero,
            [System.IntPtr]::Zero, [System.IntPtr]::Zero, 0, [ref]$outBlob)
        if (-not $ok) {
            $err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
            throw "CryptUnprotectData 失败,Win32 错误码: $err"
        }
        $plain = New-Object byte[] $outBlob.cbData
        [System.Runtime.InteropServices.Marshal]::Copy($outBlob.pbData, $plain, 0, $outBlob.cbData)
        return ,$plain
    } finally {
        if ($inBlob.pbData -ne [System.IntPtr]::Zero) {
            [System.Runtime.InteropServices.Marshal]::FreeHGlobal($inBlob.pbData)
        }
        if ($outBlob.pbData -ne [System.IntPtr]::Zero) {
            [QoderMig.Native]::LocalFree($outBlob.pbData) | Out-Null
        }
    }
}

# ============== AES-256-GCM 加/解密 ==============
function Invoke-AesGcm {
    param(
        [byte[]]$Key,
        [byte[]]$Nonce,
        [byte[]]$Input,   # Encrypt 时是明文,Decrypt 时是含 16 字节 tag 的密文
        [switch]$Decrypt
    )

    if ($PSVersionTable.PSVersion.Major -lt 7) {
        throw '本脚本需要 PowerShell 7+ (.NET 6+ 的 AesGcm)。' +
              "`n请运行: winget install Microsoft.PowerShell" +
              "`n然后用 pwsh 而不是 powershell 启动。"
    }
    Add-Type -AssemblyName System.Security.Cryptography -ErrorAction Stop
    $gcm = [System.Security.Cryptography.AesGcm]::new($Key)

    if ($Decrypt) {
        if ($Input.Length -lt 16) { throw '密文太短,不可能含 16 字节 tag' }
        $tag = $Input[-16..-1]
        $ct  = $Input[0..($Input.Length - 17)]
        $pt  = New-Object byte[] $ct.Length
        $gcm.Decrypt($Nonce, $ct, $tag, $pt, $null)
        return ,$pt
    } else {
        $ct  = New-Object byte[] $Input.Length
        $tag = New-Object byte[] 16
        $gcm.Encrypt($Nonce, $ct, $tag, $Input, $null)
        $out = New-Object byte[] ($ct.Length + $tag.Length)
        [System.Buffer]::BlockCopy($ct,  0, $out, 0,        $ct.Length)
        [System.Buffer]::BlockCopy($tag, 0, $out, $ct.Length, $tag.Length)
        return ,$out
    }
}

# ============== 取 AES-256 master key ==============
function Get-QoderEncryptionKey {
    param([string]$LocalStatePath)

    if (-not (Test-Path -LiteralPath $LocalStatePath)) {
        throw "未找到 Local State: $LocalStatePath"
    }
    $json = Get-Content -LiteralPath $LocalStatePath -Raw -Encoding UTF8 |
            ConvertFrom-Json
    $b64  = $json.os_crypt.encrypted_key
    if (-not $b64) { throw 'Local State 缺少 os_crypt.encrypted_key' }

    $bytes = [System.Convert]::FromBase64String($b64)
    $prefix = -join ($bytes[0..4] | ForEach-Object { [char]$_ })
    if ($prefix -ne 'DPAPI') {
        throw "encrypted_key 头 5 字节不是 DPAPI,实际: $prefix"
    }
    $dpapiBlob = $bytes[5..($bytes.Length - 1)]
    $key = Invoke-DpapiUnprotect -Cipher $dpapiBlob
    if ($key.Length -ne 32) {
        throw "DPAPI 解出的 AES key 长度异常: $($key.Length) (期望 32)"
    }
    return ,$key
}

# ============== SQLite 读 state.vscdb ==============
function Read-StateVscdbSecret {
    param(
        [string]$DbPath,
        [string]$Key
    )

    if (-not (Test-Path -LiteralPath $DbPath)) {
        return $null
    }

    # 先复制到临时文件避开 WAL 锁
    $tmpDir  = Join-Path $env:TEMP ("qoder-mig-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
    $tmpDb = Join-Path $tmpDir 'state.vscdb'
    try {
        Copy-Item -LiteralPath $DbPath -Destination $tmpDb -Force

        if ($PSVersionTable.PSVersion.Major -ge 7) {
            Add-Type -AssemblyName Microsoft.Data.Sqlite -ErrorAction Stop
            $conn = [Microsoft.Data.Sqlite.SqliteConnection]::new("Data Source=$tmpDb;Mode=ReadOnly")
            $conn.Open()
            try {
                $cmd  = $conn.CreateCommand()
                $cmd.CommandText = 'SELECT value FROM ItemTable WHERE key = $k'
                $p    = $cmd.Parameters.Add('$k', [Microsoft.Data.Sqlite.SqliteType]::Text)
                $p.Value = $Key
                $reader = $cmd.ExecuteReader()
                if ($reader.Read()) {
                    return $reader.GetString(0)
                }
            } finally { $conn.Close() }
        } else {
            # PS 5.1 fallback (.NET Framework)
            try {
                Add-Type -AssemblyName System.Data.SQLite -ErrorAction Stop
            } catch {
                throw 'PowerShell 5.1 下,需要 .NET Framework 的 System.Data.SQLite。' +
                      "`n两种解法:1)装 pwsh 7+;  2)Install-Package System.Data.SQLite -ProviderName NuGet"
            }
            $conn = New-Object System.Data.SQLite.SQLiteConnection "Data Source=$tmpDb;Version=3;Read Only=True;"
            $conn.Open()
            try {
                $cmd  = $conn.CreateCommand()
                $cmd.CommandText = 'SELECT value FROM ItemTable WHERE key = @k'
                $cmd.Parameters.AddWithValue('@k', $Key) | Out-Null
                $reader = $cmd.ExecuteReader()
                if ($reader.Read()) {
                    return $reader.GetString(0)
                }
            } finally { $conn.Close() }
        }
        return $null
    } finally {
        Remove-Item -LiteralPath $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ============== 解密 userInfo / userPlan / creditUsage secret ==============
function Decode-QoderSecret {
    param(
        [byte[]]$MasterKey,
        [string]$Base64Cipher
    )

    $envelope = $Base64Cipher | ConvertFrom-Json
    $bytes = [byte[]]@($envelope.data)

    if ($bytes.Length -lt 15 -or $bytes[0] -ne [byte]'v' -or $bytes[1] -ne [byte]'1' -or $bytes[2] -ne [byte]'0') {
        throw "密文前缀不是 v10,实际: $($bytes[0..2] -join ',')"
    }
    $nonce  = $bytes[3..14]                    # 12 字节
    $cipher = $bytes[15..($bytes.Length - 1)]  # ciphertext + 16 字节 tag

    $plain = Invoke-AesGcm -Key $MasterKey -Nonce $nonce -Input $cipher -Decrypt
    return [System.Text.Encoding]::UTF8.GetString($plain)
}

# ============== 加密(注入用) ==============
function Encode-QoderSecret {
    param(
        [byte[]]$MasterKey,
        [string]$PlainJson
    )
    $plainBytes = [System.Text.Encoding]::UTF8.GetBytes($PlainJson)
    $nonce  = New-Object byte[] 12
    $rng    = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    $rng.GetBytes($nonce)

    $ctWithTag = Invoke-AesGcm -Key $MasterKey -Nonce $nonce -Input $plainBytes

    $envelope = New-Object byte[] ($V10_PREFIX.Length + $nonce.Length + $ctWithTag.Length)
    $pos = 0
    [System.Buffer]::BlockCopy($V10_PREFIX, 0, $envelope, $pos, $V10_PREFIX.Length); $pos += $V10_PREFIX.Length
    [System.Buffer]::BlockCopy($nonce,      0, $envelope, $pos, $nonce.Length);      $pos += $nonce.Length
    [System.Buffer]::BlockCopy($ctWithTag,  0, $envelope, $pos, $ctWithTag.Length)

    # 转成 SQLite 期待的 {"type":"Buffer","data":[...]} 形态
    return ([pscustomobject]@{
        type = 'Buffer'
        data = [byte[]]$envelope
    } | ConvertTo-Json -Compress)
}

# ============== 写 ItemTable(注入用) ==============
function Update-ItemTableRow {
    param(
        [string]$DbPath,
        [string]$Key,
        [string]$Base64Cipher
    )

    if ($PSVersionTable.PSVersion.Major -ge 7) {
        Add-Type -AssemblyName Microsoft.Data.Sqlite -ErrorAction Stop
        $conn = [Microsoft.Data.Sqlite.SqliteConnection]::new("Data Source=$DbPath")
        $conn.Open()
        try {
            $cmd = $conn.CreateCommand()
            $cmd.CommandText = 'INSERT OR REPLACE INTO ItemTable (key, value) VALUES ($k, $v)'
            $p1 = $cmd.Parameters.Add('$k', [Microsoft.Data.Sqlite.SqliteType]::Text);  $p1.Value = $Key
            $p2 = $cmd.Parameters.Add('$v', [Microsoft.Data.Sqlite.SqliteType]::Text);  $p2.Value = $Base64Cipher
            $cmd.ExecuteNonQuery() | Out-Null
        } finally { $conn.Close() }
    } else {
        Add-Type -AssemblyName System.Data.SQLite -ErrorAction Stop
        $conn = New-Object System.Data.SQLite.SQLiteConnection "Data Source=$DbPath;Version=3;"
        $conn.Open()
        try {
            $cmd = $conn.CreateCommand()
            $cmd.CommandText = 'INSERT OR REPLACE INTO ItemTable (key, value) VALUES (@k, @v)'
            $cmd.Parameters.AddWithValue('@k', $Key)          | Out-Null
            $cmd.Parameters.AddWithValue('@v', $Base64Cipher) | Out-Null
            $cmd.ExecuteNonQuery() | Out-Null
        } finally { $conn.Close() }
    }
    Write-Host "       UPDATE $Key OK" -ForegroundColor Green
}

# ============== 提取模式(A 机) ==============
function Invoke-Extract {
    param($Paths, [string]$OutputJson)

    Write-Host "[1/4] 读取 Local State 并 DPAPI 解密 AES master key ..." -ForegroundColor Cyan
    $masterKey = Get-QoderEncryptionKey -LocalStatePath $Paths.LocalState
    Write-Host "       OK,32 字节 AES-256 key 已就绪" -ForegroundColor Green

    Write-Host "[2/4] 从 state.vscdb 读取 userInfo 密文 ..." -ForegroundColor Cyan
    $rawUserInfo = Read-StateVscdbSecret -DbPath $Paths.StateDb -Key $SECRET_USER_INFO
    if (-not $rawUserInfo) {
        throw "未在 state.vscdb 找到 $SECRET_USER_INFO —— Qoder 未登录,或已退出登录。"
    }

    Write-Host "[3/4] AES-256-GCM 解密 userInfo ..." -ForegroundColor Cyan
    $userInfoJson = Decode-QoderSecret -MasterKey $masterKey -Base64Cipher $rawUserInfo
    $userInfo     = $userInfoJson | ConvertFrom-Json

    Write-Host "[4/4] 顺手抓 userPlan / creditUsage(可空)..." -ForegroundColor Cyan
    $rawPlan  = Read-StateVscdbSecret -DbPath $Paths.StateDb -Key $SECRET_USER_PLAN
    $rawUsage = Read-StateVscdbSecret -DbPath $Paths.StateDb -Key $SECRET_CREDIT
    $planJson  = if ($rawPlan)  { (Decode-QoderSecret -MasterKey $masterKey -Base64Cipher $rawPlan ) | ConvertFrom-Json } else { $null }
    $usageJson = if ($rawUsage) { (Decode-QoderSecret -MasterKey $masterKey -Base64Cipher $rawUsage) | ConvertFrom-Json } else { $null }

    $email        = $userInfo.email
    $userId       = $userInfo.id
    $accessToken  = $userInfo.token
    $refreshToken = $userInfo.refreshToken
    $expireTime   = $userInfo.expireTime
    $refreshExp   = $userInfo.refreshTokenExpireTime

    if (-not $email)        { throw 'userInfo 缺少 email' }
    if (-not $refreshToken) { throw 'userInfo 缺少 refreshToken —— 可能账号没完整登录' }

    $output = [ordered]@{
        schema                  = 'cockpit-tools/qoder-migrate/v1'
        extractedAt             = (Get-Date).ToString('o')
        email                   = $email
        userId                  = $userId
        accessToken             = $accessToken
        refreshToken            = $refreshToken
        expireTime              = $expireTime
        refreshTokenExpireTime  = $refreshExp
        # 下面是 cockpit-tools 注入链路期望的完整快照
        # 字段名严格对齐 qoder_account.rs 中的 QoderAccount 字段
        authUserInfoRaw         = $userInfo
        authUserPlanRaw         = $planJson
        authCreditUsageRaw      = $usageJson
    }
    $output | ConvertTo-Json -Depth 10 |
        Out-File -LiteralPath $OutputJson -Encoding UTF8

    Write-Host ""
    Write-Host "提取完成" -ForegroundColor Green
    Write-Host "  email        : $email"
    Write-Host "  userId       : $userId"
    Write-Host "  access_token : $($accessToken.Substring(0, [Math]::Min(24, $accessToken.Length)))... (len=$($accessToken.Length))"
    Write-Host "  refresh_token: $($refreshToken.Substring(0, [Math]::Min(24, $refreshToken.Length)))... (len=$($refreshToken.Length))"
    Write-Host ""
    Write-Host "  -> 已写入: $OutputJson" -ForegroundColor Yellow
    Write-Host "  -> 下一步: 把这个文件传到 B 机,然后在 B 机执行 inject。" -ForegroundColor Yellow
}

# ============== 注入模式(B 机) ==============
function Invoke-Inject {
    param($Paths, [string]$InputJson)

    if (-not (Test-Path -LiteralPath $InputJson)) {
        throw "找不到输入文件: $InputJson"
    }
    $creds = Get-Content -LiteralPath $InputJson -Raw -Encoding UTF8 | ConvertFrom-Json

    Write-Host "[1/5] 校验导入 JSON ..." -ForegroundColor Cyan
    if (-not $creds.refreshToken) { throw 'JSON 缺少 refreshToken' }
    Write-Host "       email = $($creds.email)" -ForegroundColor Green
    Write-Host "       准备注入到: $($Paths.StateDb)" -ForegroundColor Green

    Write-Host "[2/5] 检查 Qoder 进程 ..." -ForegroundColor Cyan
    $qoderProcs = Get-Process -Name 'Qoder','Qoder.exe','qoder' -ErrorAction SilentlyContinue
    if ($qoderProcs) {
        Write-Host "       检测到运行中的 Qoder 进程:" -ForegroundColor Yellow
        $qoderProcs | ForEach-Object { Write-Host "         PID=$($_.Id)  $($_.ProcessName)" -ForegroundColor Yellow }
        throw '请先关闭 Qoder IDE,然后重新运行 inject 模式。' +
              "`n(脚本不强制 kill,杀进程会触发 Qoder 自身的加密重写,可能让刚写的数据被覆盖)"
    }
    Write-Host "       没有运行中的 Qoder 进程" -ForegroundColor Green

    Write-Host "[3/5] 用 B 机 DPAPI 重新生成 master key 缓存 ..." -ForegroundColor Cyan
    $masterKey = Get-QoderEncryptionKey -LocalStatePath $Paths.LocalState
    Write-Host "       OK,B 机 32 字节 master key 就绪" -ForegroundColor Green

    Write-Host "[4/5] 用 B 机 master key 重新加密 userInfo / userPlan / creditUsage ..." -ForegroundColor Cyan
    $reEncUser = Encode-QoderSecret -MasterKey $masterKey -PlainJson ($creds.authUserInfoRaw    | ConvertTo-Json -Depth 10 -Compress)
    $reEncPlan = if ($creds.authUserPlanRaw)    { Encode-QoderSecret -MasterKey $masterKey -PlainJson ($creds.authUserPlanRaw    | ConvertTo-Json -Depth 10 -Compress) } else { $null }
    $reEncUse  = if ($creds.authCreditUsageRaw) { Encode-QoderSecret -MasterKey $masterKey -PlainJson ($creds.authCreditUsageRaw | ConvertTo-Json -Depth 10 -Compress) } else { $null }

    Write-Host "[5/5] 写回 state.vscdb 的 ItemTable ..." -ForegroundColor Cyan
    $parent = Split-Path -Parent $Paths.StateDb
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    Update-ItemTableRow -DbPath $Paths.StateDb -Key $SECRET_USER_INFO -Base64Cipher $reEncUser
    if ($reEncPlan) { Update-ItemTableRow -DbPath $Paths.StateDb -Key $SECRET_USER_PLAN -Base64Cipher $reEncPlan }
    if ($reEncUse)  { Update-ItemTableRow -DbPath $Paths.StateDb -Key $SECRET_CREDIT    -Base64Cipher $reEncUse  }

    Write-Host ""
    Write-Host "注入完成" -ForegroundColor Green
    Write-Host "  email        : $($creds.email)"
    Write-Host "  state.vscdb  : $($Paths.StateDb)"
    Write-Host ""
    Write-Host "  -> 现在启动 Qoder IDE,它会认为 $($creds.email) 已登录。" -ForegroundColor Yellow
    Write-Host "  -> 德姨提示:不要同时启动 A 机和 B 机的 cockpit-tools 刷新配额(跨机指纹可能冲突)。" -ForegroundColor Yellow
}

# ============== Show(默认) ==============
function Invoke-Show {
    Write-Host 'Qoder 跨机迁移助手 (纯 PowerShell 零依赖版)' -ForegroundColor Cyan
    Write-Host ''
    Write-Host '要求:PowerShell 7+ (因为 .NET 6+ 的 AesGcm 是 GCM 唯一原生路径)。'
    Write-Host '  安装:winget install Microsoft.PowerShell'
    Write-Host ''
    Write-Host '用法:'
    Write-Host '  A 机: pwsh -File .\QoderMigrate.ps1 -Mode extract -OutputJson .\qoder_credentials.json'
    Write-Host '  B 机: pwsh -File .\QoderMigrate.ps1 -Mode inject  -InputJson  .\qoder_credentials.json'
    Write-Host ''
    Write-Host '对照源码(德姨抄的):'
    Write-Host '  - state.vscdb     -> src-tauri/src/modules/qoder_instance.rs:80-82'
    Write-Host '  - secret key 名   -> src-tauri/src/modules/qoder_account.rs:13-15'
    Write-Host '  - Local State     -> src-tauri/src/modules/vscode_paths.rs:91-93'
    Write-Host '  - DPAPI + GCM     -> src-tauri/src/modules/vscode_inject.rs:153-263'
    Write-Host ''
    $paths = Get-QoderPaths -Override $QoderUserDataDir
    Write-Host "当前机器 Qoder 路径扫描:"
    foreach ($p in $paths.PSObject.Properties) {
        $exists = Test-Path -LiteralPath $p.Value
        $color  = if ($exists) { 'Green' } else { 'DarkGray' }
        $marker = if ($exists) { '[✓]' } else { '[ ]' }
        Write-Host ("  {0} {1,-12} : {2}" -f $marker, $p.Name, $p.Value) -ForegroundColor $color
    }
}

# ============== 入口 ==============
$paths = Get-QoderPaths -Override $QoderUserDataDir
switch ($Mode) {
    'extract' { Invoke-Extract -Paths $paths -OutputJson $OutputJson }
    'inject'  { Invoke-Inject  -Paths $paths -InputJson  $InputJson  }
    default   { Invoke-Show }
}
