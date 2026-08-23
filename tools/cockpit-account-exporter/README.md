# Cockpit Account Exporter

这是一个与 Cockpit 桌面程序、Tauri、Sidecar、1456 API 服务和 Python 环境完全解耦的离线导出器。

它直接只读 Cockpit 本机数据目录，解密 Codex 账号详情，归一化额度窗口和重置时间，并合并本地 API 网关的逐账号用量统计。运行过程中不会联网、不会刷新 Token、不会修改账号，也不会读取 `codex_local_access_sidecar/auths`。

## 导出内容

默认只导出 Team/Business/Enterprise 家族账号，真实身份字段不打码：

- 完整邮箱；
- Cockpit 内部账号 ID；
- ChatGPT Account ID；
- Organization ID；
- User ID；
- Workspace 名称和结构；
- 套餐、订阅到期时间和授权状态；
- 主/次额度窗口、额外模型窗口、Code Review 窗口；
- 剩余/已用百分比、窗口长度、Unix/UTC/本地重置时间；
- 每日、本周、本月的请求数、成功/失败数和 Token 用量；
- Reset Credits 的安全元数据；
- 每个产物的 SHA-256。

导出器明确排除：

- ID/Access/Refresh Token；
- OpenAI API Key；
- Agent Identity 私钥和 Task ID；
- 账号密码和两步验证密钥；
- 邮箱查询 URL、手机号；
- 上游原始 `raw_data` 和原始错误正文。

这些字段不是“脱敏后的账号身份”，而是可直接接管账号的凭据，不属于用量导出数据。

## 环境要求

- Node.js 18 或更高版本；
- Windows PowerShell 5.1 或 PowerShell 7（使用包装脚本时）；
- 必须在保存 Cockpit 数据目录和 `secure-account-storage.key` 的原计算机上运行。

工具没有 npm 依赖，不需要执行 `npm install`。

## 页面操作前端

在仓库根目录双击或运行：

```powershell
.\tools\cockpit-account-exporter\open-export-page.cmd
```

启动器会选择一个空闲的本机端口并打开默认浏览器。页面可以选择：

- 正式库或开发库；
- Team 家族、Pro、Plus、Free 或全部套餐；
- 账号身份与生命周期、额度窗口与重置时间、本地 API 网关用量；
- JSON、CSV 或同时生成；
- 快照陈旧阈值；
- 自定义只读数据目录和输出根目录；
- 是否显式允许跳过损坏账号。

页面提供“只验证”和“执行导出”两个操作。“只验证”不会创建输出目录；页面结果区只显示安全摘要、输出路径、文件名和 SHA-256，不会显示账号正文。

本地 Web 服务只绑定 `127.0.0.1`，默认使用动态空闲端口；API 同时校验精确 Host、HttpOnly/SameSite 会话 Cookie 和同源 Origin，不启用 CORS。页面中的“关闭服务”按钮或启动终端中的 `Ctrl+C` 都可以停止服务。

指定固定本机端口或禁止自动打开浏览器：

```powershell
.\tools\cockpit-account-exporter\open-export-page.cmd `
    -Port 14661 `
    -NoOpen
```

## 最简单的 Windows 用法

在仓库根目录运行：

```powershell
.\tools\cockpit-account-exporter\export-team-usage.cmd
```

或者直接运行 PowerShell 包装器：

```powershell
powershell.exe `
    -NoProfile `
    -ExecutionPolicy Bypass `
    -File '.\tools\cockpit-account-exporter\Export-CockpitAccountData.ps1'
```

默认读取：

```text
%USERPROFILE%\.antigravity_cockpit
```

默认写入：

```text
.\cockpit-account-exports\account-export-production-YYYYMMDD-HHMMSS\
```

PowerShell 包装器会在写入任何导出数据之前，把导出目录 ACL 限制为当前用户、SYSTEM 和本机 Administrators。

## 输出文件

```text
account-export-production-YYYYMMDD-HHMMSS\
├── cockpit-account-export.json
├── accounts.csv
├── quota-windows.csv
├── gateway-usage.csv
└── export-summary.json
```

| 文件 | 用途 |
|---|---|
| `cockpit-account-export.json` | 完整、嵌套、字段不打码的机器可读结果 |
| `accounts.csv` | 账号身份、套餐、订阅、授权和生命周期状态 |
| `quota-windows.csv` | 每个账号的所有额度窗口和重置时间，一行一个窗口 |
| `gateway-usage.csv` | 每个账号每日/本周/本月的本地 API 用量 |
| `export-summary.json` | 账号数、异常数、SHA-256 和导出版本，不含凭据 |

`export-summary.json` 始终生成；其余文件只在对应数据集和输出格式被选中时生成。选择性 JSON 会省略未选数据集的详细字段，但仍保留账号 ID、邮箱和套餐等必要关联字段。

CSV 使用 UTF-8 BOM，Excel 可直接打开；以 `= + - @` 等字符开头的文本会加 Excel 安全前缀，防止公式注入。JSON 中保存的原始身份值不会被更改。

## 常用参数

导出所有套餐：

```powershell
.\tools\cockpit-account-exporter\export-team-usage.cmd `
    -PlanFamily all
```

单独导出开发 profile：

```powershell
.\tools\cockpit-account-exporter\export-team-usage.cmd `
    -Profile development
```

指定数据目录和输出根目录：

```powershell
.\tools\cockpit-account-exporter\export-team-usage.cmd `
    -DataDirectory 'D:\CockpitData' `
    -OutputRoot 'D:\PrivateExports'
```

只验证、不落盘：

```powershell
.\tools\cockpit-account-exporter\export-team-usage.cmd `
    -ValidateOnly
```

只生成 CSV：

```powershell
.\tools\cockpit-account-exporter\export-team-usage.cmd `
    -Format csv
```

只生成额度窗口相关文件：

```powershell
.\tools\cockpit-account-exporter\export-team-usage.cmd `
    -Datasets 'quota'
```

同时生成账号清单和网关用量，但不生成额度窗口文件：

```powershell
.\tools\cockpit-account-exporter\export-team-usage.cmd `
    -Datasets 'accounts,gateway'
```

默认遇到任何损坏或无法解密的账号时会整体失败，避免形成“看似完整、实际漏账号”的结果。如果明确接受部分结果，可使用：

```powershell
.\tools\cockpit-account-exporter\export-team-usage.cmd `
    -SkipInvalid
```

所有跳过项都会写入 `export-summary.json`。

## 直接使用 Node CLI

Windows 上推荐优先使用前面的 `.cmd` 或 PowerShell 包装器。直接调用 Node CLI 不会运行 `icacls`，因此输出目录会继承父目录的 Windows ACL；只有在目标目录本身已经受控时才建议这样运行。

```powershell
node.exe `
    '.\tools\cockpit-account-exporter\cockpit-account-exporter.cjs' `
    --profile production `
    --plan-family team `
    --format both `
    --datasets accounts,quota,gateway `
    --output-dir '.\private-export'
```

查看帮助：

```powershell
node.exe `
    '.\tools\cockpit-account-exporter\cockpit-account-exporter.cjs' `
    --help
```

## 测试

从仓库根目录运行：

```powershell
node.exe --test `
    '.\tools\cockpit-account-exporter\test\*.test.cjs'
```

测试使用临时伪造账号和临时 AES-256-GCM 密钥，不会读取真实账号库。

## 安全边界

- 命令行导出模式不监听端口；页面模式只监听 `127.0.0.1`，不绑定局域网或公网地址；
- 工具不调用 `chatgpt.com` 或任何网络地址；
- 工具不加载 API 服务 Key；
- 工具不读取 Sidecar OAuth 投影；
- 工具不写入 Cockpit 数据目录；
- 工具只在内存中持有解密后的账号对象；
- 输出目录必须是空目录，拒绝覆盖已有文件；
- PowerShell 包装器和页面模式都在写入身份数据之前收紧 Windows ACL；
- 页面 API 使用精确 Host、同源 Origin 和随机会话 Cookie 防护，不返回解密后的账号对象；
- JSON/CSV 都通过允许字段映射生成，而不是直接序列化完整 `CodexAccount`。

导出文件包含完整邮箱和账号标识。不要把目录上传到公开仓库、群聊或不可信云盘；不再需要时应删除或移动到受控加密存储。
