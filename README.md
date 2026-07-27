# Cockpit Tools

[English](README.en.md) · [Portuguese (BR)](README.pt-br.md) · 简体中文

[![GitHub stars](https://img.shields.io/github/stars/jlcodes99/cockpit-tools?style=flat&color=gold)](https://github.com/jlcodes99/cockpit-tools)
[![GitHub downloads](https://img.shields.io/github/downloads/jlcodes99/cockpit-tools/total?style=flat&color=blue)](https://github.com/jlcodes99/cockpit-tools/releases)
[![GitHub release](https://img.shields.io/github/v/release/jlcodes99/cockpit-tools?style=flat)](https://github.com/jlcodes99/cockpit-tools/releases)
[![GitHub issues](https://img.shields.io/github/issues/jlcodes99/cockpit-tools)](https://github.com/jlcodes99/cockpit-tools/issues)

一款**通用的 AI IDE 账号管理工具**，目前支持 **Antigravity IDE**、**Codex**、**GitHub Copilot**、**Windsurf**、**Kiro**、**Cursor**、**Grok CLI**、**CodeBuddy**、**CodeBuddy CN**、**Qoder**、**Trae**、**TRAE SOLO**、**Trae CN**、**TRAE SOLO CN**、**Zed** 和 **ZCode**，并支持多账号多实例并行运行。


> 本工具旨在帮助用户高效管理多个 AI IDE 账号，支持一键切换、配额监控、自动唤醒与应用多开并行运行，助您充分利用不同账号的资源。

**功能**：一键切号 · 多账号管理 · 应用多开 · 配额监控 · 唤醒任务 · 插件联动 · GitHub Copilot 管理 · Windsurf 管理 · Kiro 管理 · Cursor 管理 · Grok CLI 管理 · CodeBuddy 管理 · CodeBuddy CN 管理 · Qoder 管理 · Trae 套件管理 · Zed 管理 · ZCode 管理

**语言**：支持 18 种语言

🇺🇸 English · 🇨🇳 简体中文 · 繁體中文 · 🇯🇵 日本語 · 🇩🇪 Deutsch · 🇪🇸 Español · 🇫🇷 Français · 🇮🇹 Italiano · 🇰🇷 한국어 · 🇧🇷 Português · 🇷🇺 Русский · 🇹🇷 Türkçe · 🇵🇱 Polski · 🇨🇿 Čeština · 🇸🇦 العربية · 🇻🇳 Tiếng Việt · 🇮🇩 Bahasa Indonesia

**官方支持平台**：macOS、Windows、Linux。

---

## 赞助商

<table>
  <tr>
    <td width="120" align="center">
      <a href="https://apikey.fun/register?aff=COCKPIT">
        <img src="src/assets/icons/apikey-fun.png" alt="APIKEY.FUN" width="72" />
      </a>
    </td>
    <td>
      <a href="https://apikey.fun/register?aff=COCKPIT"><strong>APIKEY.FUN</strong></a> 是一家专业的企业级 AI 中转站，致力于为企业和个人开发者提供稳定、高效、低成本的 AI 模型 API 接入服务。平台支持 Claude、OpenAI、Gemini 等主流热门模型，价格低至官方原价的 7%。通过本项目 <a href="https://apikey.fun/register?aff=COCKPIT"><strong>专属链接</strong></a> 注册，还可享受最高 <strong>充值永久 95 折</strong> 专属优惠。
    </td>
  </tr>
  <tr>
    <td width="120" align="center">
      <a href="https://roxybrowser.cn?code=0326VTDA">
        <img src="src/assets/icons/roxybrowser.jpg" alt="RoxyBrowser" width="96" />
      </a>
    </td>
    <td>
      <a href="https://roxybrowser.cn?code=0326VTDA"><strong>RoxyBrowser（Roxy浏览器）</strong></a> 是面向多账号运营与 AI 自动化场景的指纹浏览器，支持独立浏览器指纹环境、Cookie / 存储隔离、Roxy 原生住宅 IP、团队协作与 API / MCP 自动化能力，适合需要管理 AI 账号矩阵、降低账号关联风险、提升长期使用稳定性的用户。通过 Cockpit <a href="https://roxybrowser.cn?code=0326VTDA"><strong>邀请链接</strong></a> 注册或购买，可享受 10% 粉丝折扣。
    </td>
  </tr>
</table>

---

## 功能概览

### 1. 仪表盘 (Dashboard)

全新的可视化仪表盘，为您提供一站式的状态概览：

- **十六平台支持**：同时展示 Antigravity IDE、Codex、GitHub Copilot、Windsurf、Kiro、Cursor、Grok CLI、CodeBuddy、CodeBuddy CN、Qoder、Trae、TRAE SOLO、Trae CN、TRAE SOLO CN、Zed 与 ZCode 的账号状态
- **配额监控**：实时查看各模型剩余配额、重置时间
- **快捷操作**：一键刷新、一键唤醒
- **可视化进度**：直观的进度条展示配额消耗情况

> ![Dashboard Overview](docs/images/dashboard_overview.png)

### 2. Antigravity IDE 账号管理

- **一键切号**：一键切换当前使用的账号，无需手动登录登出
- **多种导入**：支持 OAuth 授权、Refresh Token、插件同步
- **唤醒任务**：定时唤醒 AI 模型，提前触发配额重置周期

> ![Antigravity IDE Accounts](docs/images/antigravity_list.png)
>
> *(唤醒任务)*
> ![Wakeup Tasks](docs/images/wakeup_detail.png)

#### 2.1 Antigravity IDE 应用多开

支持同一平台多账号多实例并行运行。比如同时打开两个 Antigravity IDE，分别绑定不同账号，分别处理不同项目，互不影响。

- **独立账号**：每个实例绑定不同账号并独立运行
- **并行项目**：多实例同时处理不同任务/项目
- **参数隔离**：支持自定义实例目录与启动参数

> ![Antigravity IDE Instances](docs/images/antigravity_instances.png)

### 3. Codex 账号管理

- **专属支持**：专为 Codex 优化的账号管理体验
- **配额展示**：清晰展示 Hourly 和 Weekly 配额状态
- **计划识别**：自动识别账号 Plan 类型 (Basic, Plus, Team 等)
- **API 服务**：本地 Codex API 服务由内置 CLIProxyAPI sidecar 驱动，Cockpit Tools 负责账号同步、配置投影、状态与用量统计；Base URL、API Key 与用户操作方式保持不变。

> ![Codex Accounts](docs/images/codex_list.png)

#### 3.1 Codex 应用多开

Codex 同样支持多账号多实例并行运行。比如同时打开两个 Codex，分别绑定不同账号，分别处理不同项目，互不影响。

- **独立账号**：每个实例绑定不同账号并独立运行
- **并行项目**：多实例同时处理不同任务/项目
- **参数隔离**：支持自定义实例目录与启动参数

> ![Codex Instances](docs/images/codex_instances.png)

### 4. GitHub Copilot 账号管理

- **账号导入**：支持 OAuth 授权、Token/JSON 导入
- **配额视图**：展示 Inline Suggestions / Chat messages 使用情况与重置时间
- **订阅识别**：自动识别 Free / Individual / Pro / Business / Enterprise 等计划类型
- **批量管理**：支持标签与批量操作

#### 4.1 GitHub Copilot 应用多开

基于 VS Code 的 Copilot 多实例管理，支持独立配置与生命周期控制。

- **独立配置**：每个实例拥有独立的用户目录
- **快速启停**：一键启动/停止/强制关闭实例
- **窗口管理**：支持打开实例窗口与批量关闭

### 5. Windsurf 账号管理

- **账号导入**：支持 OAuth 授权、Token/JSON 导入与本地导入
- **配额视图**：展示 Plan、User Prompt credits、Add-on prompt credits 与周期信息
- **批量管理**：支持标签与批量操作
- **切号注入**：支持切号后注入并启动 Windsurf

#### 5.1 Windsurf 应用多开

支持 Windsurf 多实例管理，支持独立配置与生命周期控制。

- **独立配置**：每个实例拥有独立的用户目录
- **快速启停**：一键启动/停止/强制关闭实例
- **窗口管理**：支持打开实例窗口与批量关闭

### 6. Kiro 账号管理

- **账号导入**：支持 OAuth 授权、Token/JSON 导入与本地导入
- **配额视图**：展示 Plan、User Prompt credits、Add-on prompt credits 与周期信息
- **批量管理**：支持标签与批量操作
- **切号注入**：支持切号后注入并启动 Kiro

#### 6.1 Kiro 应用多开

支持 Kiro 多实例管理，支持独立配置与生命周期控制。

- **独立配置**：每个实例拥有独立的用户目录
- **快速启停**：一键启动/停止/强制关闭实例
- **窗口管理**：支持打开实例窗口与批量关闭

### 7. Cursor 账号管理

- **账号导入**：支持 OAuth 授权、Token/JSON 导入与本地导入
- **配额视图**：展示 Total Usage、Auto + Composer、API Usage、On-Demand 与周期信息
- **批量管理**：支持标签与批量操作
- **切号注入**：支持切号后注入并启动 Cursor

#### 7.1 Cursor 应用多开

支持 Cursor 多实例管理，支持独立配置与生命周期控制。

- **独立配置**：每个实例拥有独立的用户目录
- **快速启停**：一键启动/停止/强制关闭实例
- **窗口管理**：支持打开实例窗口与批量关闭


### 8. Grok CLI 账号管理

- **OAuth 授权**：支持 xAI 官方 OIDC device flow，在浏览器完成验证后自动保存账号
- **导入与脱敏导出**：可从默认 `~/.grok/auth.json` 或指定 JSON 导入官方凭据；账号页导出及通用账号备份均不含 access token/refresh token，不能用于恢复登录，迁移时需单独导入官方 `auth.json`
- **真实切号**：将选中账号按 Grok CLI 官方 registry 格式写回默认 `~/.grok/auth.json`，并保留文件中其他 registry scope
- **配额与套餐**：查询官方 billing/user/subscriptions 接口，展示周期、用量、产品配额和套餐原始值，并记录 Grok Code 访问能力
- **Token 维护**：支持 access token 自动刷新、refresh token rotation 与配额预警

#### 8.1 Grok CLI 多实例

Grok CLI 默认实例直接沿用官方 `~/.grok` 目录，启动时不设置 `GROK_HOME`；只有受管实例才使用独立目录并为该实例设置独立 `GROK_HOME`。

- **账号绑定**：默认实例可跟随当前账号，每个受管实例可绑定不同账号
- **运行隔离**：受管实例的 `auth.json`、工作目录与启动参数互相独立
- **终端启停**：支持生成或执行终端启动命令、停止实例与批量关闭
- **目录保护**：非默认实例目录仅允许位于默认受管根目录；删除时移入回收站，历史配置中的外部目录只解除登记且不会被写入或删除

### 9. CodeBuddy 账号管理

- **账号导入**：支持 OAuth 授权、Token/JSON 导入
- **配额视图**：支持配额查询、周期信息与加量包展示
- **批量管理**：支持标签与批量操作
- **切号注入**：支持切号后注入并启动 CodeBuddy

#### 8.1 CodeBuddy 应用多开

支持 CodeBuddy 多实例管理，支持独立配置与生命周期控制。

- **独立配置**：每个实例拥有独立的用户目录
- **快速启停**：一键启动/停止/强制关闭实例
- **窗口管理**：支持打开实例窗口与批量关闭

### 10. CodeBuddy CN 账号管理

- **账号导入**：支持 OAuth 授权、Token/JSON 导入与本机客户端导入
- **配额视图**：展示套餐与用量状态，并支持跳转官方网页查看配额详情
- **批量管理**：支持标签与批量操作
- **切号注入**：支持切号后按客户端本地认证存储规则注入并启动 CodeBuddy CN

#### 9.1 CodeBuddy CN 应用多开

支持 CodeBuddy CN 多实例管理，支持独立配置与生命周期控制。

- **独立配置**：每个实例拥有独立的用户目录
- **快速启停**：一键启动/停止/强制关闭实例
- **窗口管理**：支持打开实例窗口与批量关闭

### 11. Qoder 账号管理

- **账号导入**：支持本机导入与 JSON 导入
- **配额视图**：展示 Credits 使用、剩余额度与套餐原始值
- **批量管理**：支持标签、筛选、导出与批量删除/刷新
- **切号注入**：支持切号后注入并启动 Qoder

#### 10.1 Qoder 应用多开

支持 Qoder 多实例管理，支持独立配置与生命周期控制。

- **独立配置**：每个实例拥有独立的用户目录
- **快速启停**：一键启动/停止/强制关闭实例
- **窗口管理**：支持打开实例窗口与批量关闭

### 12. Trae 账号管理

- **账号导入**：支持本机导入与 JSON 导入
- **配额视图**：展示套餐原始值、美元消耗/总额度与重置时间
- **批量管理**：支持标签、筛选、导出与批量删除/刷新
- **Trae 套件**：支持 Trae、TRAE SOLO、Trae CN、TRAE SOLO CN 默认客户端的本机导入与切号注入，默认归入 Trae 分组
- **切号注入**：支持切号后按各客户端真实落盘规则写回并启动目标客户端

#### 11.1 Trae 应用多开

支持原 Trae 客户端多实例管理，支持独立配置与生命周期控制。

- **独立配置**：每个实例拥有独立的用户目录
- **快速启停**：一键启动/停止/强制关闭实例
- **窗口管理**：支持打开实例窗口与批量关闭

### 13. Zed 账号管理

- **账号导入**：支持官方 OAuth 授权、JSON 导入与本机当前登录状态导入
- **配额视图**：展示订阅状态、Edit Predictions、Token Spend、Spend Limit 与账期结束时间
- **批量管理**：支持标签、筛选、导出与批量删除/刷新
- **切号注入**：支持切号后按 Zed 客户端真实落盘规则应用账号，并可按需重启官方客户端

### 14. ZCode 账号管理

- **官方登录**：关闭 ZCode 后，可在 Cockpit 内置授权窗口完成 Z.ai 或 BigModel OAuth；窗口直接拦截 `zcode://` 官方回调并保存账号
- **导入导出**：支持读取本机 `~/.zcode/v2/credentials.json` 加密凭据、JSON 导入导出与账号备份
- **配额视图**：查询订阅套餐与按模型拆分的额度，并保留套餐原始值
- **批量管理**：支持标签、搜索、套餐筛选、导出与批量删除/刷新
- **真实切号**：按 ZCode 官方凭据格式加密并回写默认客户端登录数据

#### 13.1 ZCode 应用多开

支持 ZCode 多实例管理，每个受管实例使用独立的 Electron 用户数据、会话数据和 ZCode 数据目录。

- **账号绑定**：每个实例可绑定不同账号，也可跟随当前账号
- **独立运行**：实例凭据和业务数据互相隔离
- **生命周期管理**：支持启动、停止、聚焦与批量关闭实例

### 15. 通用设置

- **个性化设置**：主题切换、语言设置、自动刷新间隔
- **平台配置**：统一管理 Grok CLI / CodeBuddy CN / Qoder / Trae 套件 / Zed / ZCode 等平台配置与配额预警

> ![Settings](docs/images/settings_page.png)

---

## 安全性与隐私（简明版）

下面是最关心的几个问题，尽量用直白语言说明：

- **这是本地桌面工具**：不需要单独注册平台账号，也不依赖项目自建云端来存你的账号列表。
- **数据主要保存在本机**：
  - `~/.antigravity_cockpit`：Antigravity IDE 账号、配置、WebSocket 状态等
  - `~/.codex`：Codex 官方当前登录 `auth.json`
  - `~/.grok`：Grok CLI 官方默认实例与当前登录 `auth.json`
  - `~/.zcode/v2`：ZCode 官方当前登录加密凭据与配额缓存
  - 系统本地应用数据目录下 `com.antigravity.cockpit-tools`：Codex / GitHub Copilot / Windsurf / Kiro / Cursor / Grok CLI / CodeBuddy / CodeBuddy CN / Qoder / Trae 套件 / Zed / ZCode 多账号数据等；Grok CLI 的账号详情、受管 profile 与实例配置也保存在此
- **Grok CLI 凭据不加密**：access token/refresh token 以明文 JSON 保存在本机，主要依赖操作系统账号隔离与本地文件权限保护；Unix 系统上凭据目录设为 `0700`、凭据文件设为 `0600`。脱敏导出不包含 token，不能作为登录备份
- **WebSocket 默认仅本机访问**：监听 `127.0.0.1`，默认端口 `19528`，可在设置中关闭或改端口。
- **什么时候会联网**：OAuth 登录、Token 刷新、配额查询、版本更新检查等官方接口请求。
- **macOS 隐私权限弹窗说明**：在 Cockpit Tools 中启动 Codex/agent 后，如果 agent 执行的 shell 命令访问桌面、文稿、下载、照片等受保护目录，macOS 可能会把权限请求显示为“Cockpit Tools 想要访问……”。这是因为这些命令是 Cockpit Tools 启动的子进程，系统会把权限归因到宿主应用；这不等同于 Cockpit Tools 主程序主动扫描这些目录。是否允许取决于你是否信任当前 agent 任务和它将要执行的命令；不确定时可以选择拒绝，或先把项目放在普通工作目录中运行。
- **实用安全建议**：
  1. 不使用插件联动时，可关闭 WebSocket 服务。
  2. 不要把用户目录直接打包分享；备份前注意脱敏 token 文件。
  3. 在公共或共用电脑上，使用后删除账号并退出应用。

## 设置项说明（小白版）

如果你只想“能用、稳定、不折腾”，优先按“推荐值”设置即可。

### 通用设置

| 设置项 | 这是做什么的（通俗） | 推荐值 | 什么时候改 |
| --- | --- | --- | --- |
| 显示语言 | 改界面文字语言 | 你最熟悉的语言 | 只在看不懂时改 |
| 应用主题 | 改亮色/暗色外观 | 跟随系统 | 长时间夜间使用可改深色 |
| 窗口关闭行为 | 点关闭按钮后的动作 | 每次询问 | 想后台常驻选“最小化到托盘” |
| Antigravity IDE 自动刷新配额 | 后台定时更新 Antigravity IDE 配额 | 5~10 分钟 | 账号多、想更实时可改 2 分钟 |
| Codex 自动刷新配额 | 后台定时更新 Codex 配额 | 5~10 分钟 | 同上 |
| GitHub Copilot 自动刷新配额 | 后台定时更新 GitHub Copilot 配额 | 5~10 分钟 | 同上 |
| Windsurf 自动刷新配额 | 后台定时更新 Windsurf 配额 | 5~10 分钟 | 同上 |
| Kiro 自动刷新配额 | 后台定时更新 Kiro 配额 | 5~10 分钟 | 同上 |
| Cursor 自动刷新配额 | 后台定时更新 Cursor 配额 | 5~10 分钟 | 同上 |
| Grok CLI 自动刷新配额 | 后台定时刷新 token 并更新配额 | 5~10 分钟 | 同上 |
| CodeBuddy 自动刷新配额 | 后台定时更新 CodeBuddy 配额 | 5~10 分钟 | 同上 |
| CodeBuddy CN 自动刷新配额 | 后台定时更新 CodeBuddy CN 配额 | 5~10 分钟 | 同上 |
| Qoder 自动刷新配额 | 后台定时更新 Qoder 配额 | 5~10 分钟 | 同上 |
| Trae 自动刷新配额 | 后台定时更新 Trae 套件账号配额 | 5~10 分钟 | 同上 |
| Zed 自动刷新配额 | 后台定时更新 Zed 配额 | 5~10 分钟 | 同上 |
| 数据目录 | 存账号与配置文件的位置 | 默认即可 | 仅用于排查、备份 |
| Antigravity IDE/Codex/VS Code/Windsurf/Kiro/Cursor/Grok CLI/CodeBuddy/CodeBuddy CN/Qoder/Trae/Zed/OpenCode 启动路径 | 指定应用可执行文件位置 | 留空（自动检测） | 自动检测失败、或你装在自定义路径时 |
| 切换 Codex 时自动重启 OpenCode | 切换 Codex 后自动同步 OpenCode 账号信息 | 使用 OpenCode 就开启；不用就关闭 | 频繁切号且需要 OpenCode 同步时开启 |

补充说明：
- 自动刷新间隔越小，请求越频繁；若你更关注稳定，间隔可适当拉大。
- 当启用“配额重置唤醒”相关任务时，部分刷新间隔会有最小值限制（界面会提示）。

### 网络服务设置

| 设置项 | 这是做什么的（通俗） | 推荐值 | 风险/注意点 |
| --- | --- | --- | --- |
| WebSocket 服务 | 给本机插件/客户端实时通信用 | 不用插件联动就关闭 | 开启后仍是本机 `127.0.0.1` 访问 |
| 首选端口 | WebSocket 监听端口 | 默认 `19528` | 若端口冲突可改，保存后需重启应用 |
| 当前运行端口 | 实际已使用端口 | 只读查看 | 配置端口被占用时会自动回退到其它端口 |

### 三套推荐配置（直接抄）

1. **稳定省心**：自动刷新 10 分钟 + WebSocket 关闭（不用插件时）+ 路径保持默认。  
2. **高频切号**：自动刷新 2~5 分钟 + 需要联动时开启 WebSocket + OpenCode 联动开启。  
3. **安全优先**：WebSocket 关闭 + 不共享用户目录 + 定期清理不再使用的账号。  

---

## 安装指南 (Installation)

### 选项 A: 手动下载 (推荐)

前往 [GitHub Releases](https://github.com/jlcodes99/cockpit-tools/releases) 下载对应系统的安装包：

*   **macOS**: `.dmg` (Apple Silicon & Intel)
*   **Windows**: `.msi` (推荐) 或 `.exe`
*   **Linux**: `.deb` (Debian/Ubuntu)、`.rpm` 或 `.AppImage` (通用)

### 选项 B: Homebrew 安装 (macOS)

> 需要先安装 Homebrew。

```bash
brew tap jlcodes99/cockpit-tools https://github.com/jlcodes99/cockpit-tools
brew install --cask cockpit-tools
```

如果遇到 macOS “应用已损坏”或无法打开，也可以使用 `--no-quarantine` 安装：

```bash
brew install --cask --no-quarantine cockpit-tools
```

如果提示已存在应用（例如：`already an App at '/Applications/Cockpit Tools.app'`），请先删除旧版本再安装：

```bash
rm -rf "/Applications/Cockpit Tools.app"
brew install --cask cockpit-tools
```

或者直接强制覆盖安装：

```bash
brew install --cask --force cockpit-tools
```

### 🛠️ 常见问题排查 (Troubleshooting)

#### macOS 提示“应用已损坏，无法打开”？
由于 macOS 的安全机制，非 App Store 下载的应用可能会触发此提示。当前开源发布流程尚未接入 Apple Developer ID 签名和公证，因此部分系统版本会显示更严格的 Gatekeeper 提示。您可以按照以下步骤快速修复：

1.  **命令行修复** (推荐):
    打开终端，执行以下命令：
    ```bash
    sudo xattr -rd com.apple.quarantine "/Applications/Cockpit Tools.app"
    ```
    > **注意**: 如果您修改了应用名称，请在命令中相应调整路径。

2.  **或者**: 在“系统设置” -> “隐私与安全性”中点击“仍要打开”。

---

## 开发与构建

### 前置要求

- Node.js v18+
- npm v9+
- Rust（Tauri 运行时）

### 安装依赖

```bash
npm install
```

### 开发模式

```bash
npm run tauri dev
```

### 构建产物

```bash
npm run tauri build
```

---

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=jlcodes99/cockpit-tools&type=Date)](https://star-history.com/#jlcodes99/cockpit-tools&Date)

---

## 💬 交流群

QQ 交流群、微信群或新建的 Telegram 畅聊群都可以加入。

新建 Telegram 畅聊群：[点击加入](https://t.me/+Y8gMv4SlZUU2MWY1)

| QQ 群 | 微信（个人） |
| :---: | :---: |
| <img src="docs/images/qq_group_20260404_183718.png" width="200" /> | <img src="docs/images/wechat_info.jpg" width="200" /> |

---

## ☕ 赞助项目

如果不介意，请 [☕ 赞赏支持一下](docs/DONATE.md)

您的每一份支持都是对开源项目最大的鼓励！无论金额大小，都代表着您对这个项目的认可。

---

## 致谢

- Antigravity 账号切号逻辑参考：[Antigravity-Manager](https://github.com/lbjlaq/Antigravity-Manager)
- Codex API 服务集成 CLIProxyAPI，Grok CLI 账号与 OAuth 实现方向亦参考其开源实现：[router-for-me/CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI)（MIT）
- Grok 图标造型参考：[LobeHub/lobe-icons](https://github.com/lobehub/lobe-icons)（MIT）
- Grok CLI 任务用量查询与兼容解析方向参考：[junhoyeo/tokscale](https://github.com/junhoyeo/tokscale)（MIT）
- Codex API 服务协议兼容方向参考：[codex-proxy](https://github.com/icebear0828/codex-proxy)
- Codex Agent Identity 导入、动态签名与 task 失效恢复方向参考：[sub2api](https://github.com/Wei-Shaw/sub2api)
- Codex Agent Identity runtime 注册协议与 Ed25519 密钥格式参考官方实现：[openai/codex](https://github.com/openai/codex)（Apache-2.0）
- Codex、Claude CLI 与 Claude Desktop Gateway 第三方供应商预设和模型映射方向参考：[CC Switch](https://github.com/farion1231/cc-switch)
- Codex 模型目录与前端模型显示思路参考：[CodexPlusPlus](https://github.com/BigPizzaV3/CodexPlusPlus)
- Claude 可选登录 helper 运行时基于：[Electron](https://github.com/electron/electron)
- 感谢 [@longwQaQ](https://github.com/longwQaQ) 贡献 Codex 模型供应商 Responses WebSocket 配置能力（[#1512](https://github.com/jlcodes99/cockpit-tools/pull/1512)）。
- 感谢 [@sqmw](https://github.com/sqmw) 在 Trae CN 账号支持方向上的贡献（OAuth/本机导入、pay v2 配额与速通展示、CN 产品类型等实现思路），已整合进统一 Trae 套件（[#1281](https://github.com/jlcodes99/cockpit-tools/pull/1281)）。

感谢项目作者的开源贡献！如果这些项目对你有帮助，也请给他们点个 ⭐ Star 支持一下！

---

## 许可证

本项目默认采用 [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/deed.zh-hans) 许可协议（署名-非商业性使用-相同方式共享）。

- 允许：个人学习、研究、非商业场景下的使用与修改（需保留署名并遵循同协议分享要求）。
- 不允许：任何未获授权的商业使用（含企业内部商业目的、对外商业服务、付费产品集成、二次分发售卖等）。
- 商业授权：如需商业使用，请联系作者获取单独书面商业授权与报价。

---

## 免责声明

本项目仅供个人学习和研究使用。使用本项目即表示您同意：

- 未获得作者书面商业授权前，不将本项目用于任何商业用途
- 承担使用本项目的所有风险和责任
- 遵守相关服务条款和法律法规

项目作者对因使用本项目而产生的任何直接或间接损失不承担责任。
