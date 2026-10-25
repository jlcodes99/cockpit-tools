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
- **ChatGPT 官方授权**：在“API 服务 → 管理成员（添加账号）”中主动选择官方通道，可在独立弹框中为已有 Codex 账号绑定 OpenAI Sign in with ChatGPT 授权。打开后自动准备并显示授权链接和手动回调输入，可复制链接、使用无痕窗口或手动提交回调；授权窗口可以取消；未加入服务的账号仅保存授权，不会自动加入。卡片和添加账号列表以紧凑的「官」标识显示官方授权绑定状态；有效绑定始终高亮，不受是否加入 API 服务影响，服务成员状态单独显示。API 服务选择“官方优先”后，优先使用已绑定的官方授权；遇到可安全重试的授权拒绝、通道或连接异常时，先尝试同账号原有的本地 OAuth 通道，再沿用当前 Key 范围内的账号池规则。“仅本地”保留明确选择；回退不解除授权，仍执行账号、模型、额度、并发与代理限制。已输出内容或执行状态不明的请求不重放。资格拒绝会暂停官方通道，检查或重新授权后可恢复；临时错误按退避时间重新尝试官方。官方通道按默认速度发送，不传递 Fast/超快等速度覆盖；此适配不视为授权拒绝，也不触发回退或暂停模型。若随后因真实异常回退本地，则保留原始速度选择，由本地上游决定是否支持。请求记录显示原始速度、官方默认速度适配、源账号、实际拒绝、预检或暂停跳过、本地尝试、上游请求 ID 与最终结果；上游回报的档位单独保留，不据此推断实际加速。鹈鹕测智和手动、定时唤醒优先使用原账号绑定的官方第三方授权，未绑定时沿用原通道，不依赖 API 服务是否启动或加入集合。测智与唤醒在已绑定但失效时仍提示处理授权，不静默改用原凭据，不换号或重复生成。服务成员始终使用原账号，成员弹框在账号名和优先级之间用下拉选择“官方优先 / 仅本地”，其余字段位置不变；未授权时选择官方会打开授权弹框，成功后更新草稿，取消保持原方式。勾选和保存集合均不弹引导，保存时统一提交成员与接入方式，关闭弹框仍保留已完成的账号授权。本版本取消启动、页面进入和普通账号添加时的官方授权自动提示及新功能提醒；没有官方授权时默认使用本地网关，已有明确接入方式仍保留。后续可在“API 服务 → 管理成员（添加账号）”中选择接入方式或进入授权管理，统计区和总览工具栏不再单独放入口。名称、套餐、额度和分组始终沿用原账号资料。不改变原 Codex 客户端登录、切号或实例绑定；两套凭据独立加密保存并自动刷新。首次授权完成后直接保存并绑定；授权及加入流程无需选择模型、唤醒或发送测试推理。移除 API 服务成员会保留授权和上次接入方式，重新加入时直接复用，必要时自动续期。授权作为原账号的附加能力，列表只保留“授权/管理授权”，管理操作打开独立弹框；管理与重新授权不会自动加入服务。切换接入方式保留服务成员及 Key 范围和优先级；解除官方授权时明确选择改用本地网关或暂停，成员继续保留。暂停仅影响当前 API 服务，两种引擎均阻止该成员处理新请求，不改客户端登录；只有显式移出服务才移除成员。检查授权不会生成回复；官网撤销后会在检查、请求或续期时识别，不提供实时撤销通知。官方通道生图固定禁止。Rust 原生引擎通过官方公共 Responses HTTP/SSE 调用，支持本地 function/custom 工具、客户端工具发现和结果回传；工具由客户端执行，网关不代为执行本地操作。客户端 WebSocket 由本地网关转为 HTTP/SSE，连续对话仅使用经过调用者与账号归属校验的本地历史，未知 `previous_response_id` 仍需补齐完整 `input`。套餐通道不提供 compact、音频或官方未开放的托管工具。未知订阅不显示 Free；本地请求与 Token 统计仅代表本服务用量，剩余套餐额度和授权在 ChatGPT 设置中查看与管理。
- **自动能力兜底**：当前账号不支持请求能力时，API 服务自动在当前 Key 已授权的账号池中寻找兼容账号，无需单独开启或填写备用名单。候选继续受模型映射、配额及并发限制；未知能力先由当前上游处理，不发起付费探测。换号会发送必要的对话内容并消耗目标账号额度，请求详情保留相关证据。仅对明确执行前拒绝、历史完整且可安全迁移的请求换号；不重放已输出或执行状态不明的请求，不丢弃私有文件、向量库、容器引用或不兼容密文。Files、向量库和容器管理接口不属于此功能。
- **API 服务**：新安装及首次升级默认使用「Rust 原生」，之后保留用户的选择，可在 API 服务页面或快捷面板切回内置 Go CLIProxyAPI sidecar。Rust 在宿主内实现 Responses 的 HTTP/SSE/WebSocket 与 compact，以及 Chat Completions、Claude、Gemini、Ollama 兼容接口，两引擎共用账号池、代理和配置。Go 继续负责协议转换、自动重试和上游传输，由 Go 管理的本地无界面辅助进程复用 Rust 会话内核，持久实例网关不依赖主窗口存活。对于 Codex 账号请求，Go 与 Rust 在同一网关范围内共享 Session、Thread、子 Agent 和上游状态的账号归属及历史；不同 profile 的实例网关保持隔离。切换引擎保留 Base URL 和 API Key，但会重启服务并可能中断活动请求。
- **请求身份对照**：Go 与 Rust 的请求记录「详情」可对照客户端原始 `Session-Id`、`Thread-Id`，以及每次上游尝试使用的请求头、账号和传输方式。关闭或清除正文快照仍保留这些本地诊断信息；客户端未提供与历史未采集分别显示，不按当前配置补填旧记录。这些头值不代表上游已接收请求。
- **自动续接上下文**：Go 的 Codex 账号请求与 Rust 默认自动尝试续接历史对话，支持切换引擎和账号，缺少本地归属记录不再单独阻断请求。调用方、账号池权限及 profile 隔离继续生效。完整 Responses 历史原样保留，包括 reasoning、compaction、工具与子 Agent 密文及 `item.id`／`call_id`；不解密、不丢弃，也不以摘要替代。完整自包含 `input` 可在授权目标账号建立新的上游 Session／Thread，保留客户端 Thread；增量请求继续使用有效上游状态、完整检查点或 WebSocket 历史，缺失上下文或上游明确拒绝时仍可能失败并保留原历史。两引擎在转发完整官方 Agent 通信工具调用前登记消息密文归属；缺少记录的历史通信密文也自动尝试续接，新的续接归属仅在有效完整响应后保存。两引擎共用本地 `codex_native_api/identity.sqlite` 明文／密文历史缓存，实例网关使用各 profile 隔离目录内的 `native-identity.sqlite`，独立于正文诊断／快照开关。缓存上限为每个检查点 8 MiB、总计 64 MiB 和 128 个检查点，淘汰不删除会话或密文归属。重放可能增加输入 token，上游对跨账号密文的接受情况仍待真实请求验证。失败不覆盖已成功代次，已成功代次在切换引擎和重启后保留；新的上游对话不改变目标账号身份。
- **语音反代**：支持 Codex backend 实时语音路由与 `/transcribe` 转写入口，并提供 `/v1/audio/transcriptions`、`/v1/audio/translations`、`/v1/audio/speech` 及无 `/v1` 的别名。OAuth 转写使用 ChatGPT backend：兼容入口支持 `file`、`language` 和 JSON／纯文本返回，`model` 仅作为兼容字段，实际模型由 backend 决定；字幕时间戳、流式转写及其他音频操作需使用支持相应接口的 API Key 供应商。原生 `/transcribe` 请求保留原有 multipart 参数，供应商侧未指定模型时使用 `gpt-4o-mini-transcribe`。请求上限为 26 MiB，超时为 2 分钟，音频内容不写入请求快照。只处理实际发送到本地网关的请求；客户端仍直连官方转写服务或自行限制麦克风时，不会因设置模型供应商而自动改走本地网关。 OAuth 转写可能被上游拒绝（包括 `403 cloudflare challenge`），本地适配无法保证上游接受。API 服务绑定的 Codex 客户端保留原生听写入口与转写流程，是否显示和可用由客户端原生能力与设置决定；Cockpit 不接管听写或强制启用麦克风。当前官方实时语音按钮仍要求 ChatGPT 登录态；网关实时语音从账号池选择 OAuth，不要求该账号就是绑定账号，尚未开放纯 API Key 实时语音路由。

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
- **API Key 与第三方接口**：支持官方 xAI API Key，也支持配置 OpenAI 兼容的第三方 `Base URL` 与模型 ID；配置写入账号专属 `config.toml`，密钥只在启动对应 CLI 进程时通过环境变量注入
- **导入与脱敏导出**：可从默认 `~/.grok/auth.json` 或指定 JSON 导入官方凭据；账号页导出及通用账号备份均不含 access token/refresh token，不能用于恢复登录，迁移时需单独导入官方 `auth.json`
- **真实切号**：将选中账号按 Grok CLI 官方 registry 格式写回默认 `~/.grok/auth.json`，并保留文件中其他 registry scope
- **配额与套餐**：查询官方 billing/user/subscriptions 接口，展示周期、用量、产品配额和套餐原始值，并记录 Grok Code 访问能力
- **Token 维护**：支持 access token 自动刷新、refresh token rotation 与配额预警

#### 8.1 Grok CLI 多实例

Grok CLI 默认实例通常直接沿用官方 `~/.grok` 目录，启动时不设置 `GROK_HOME`；受管实例使用独立目录。API Key 账号（含第三方接口）为避免被官方 OAuth session 抢占凭据优先级，也始终通过账号专属 `GROK_HOME` 启动。

- **账号绑定**：默认实例可跟随当前账号，每个受管实例可绑定不同账号
- **运行隔离**：受管实例的 `auth.json`、`config.toml`、工作目录与启动参数互相独立
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
- **Trae 套件**：支持 Trae、TRAE SOLO、Trae CN、TRAE SOLO CN 默认客户端的本机导入与切号注入，默认独立展示，可在顶部布局中手动分组
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
  - `~/.cockpit_tools`：各平台账号、公共配置、日志、备份与实例配置；开发环境使用 `~/.cockpit_tools_dev`，`COCKPIT_TOOLS_DATA_DIR` 可指定自定义目录
  - 已有安装会在后台自动为 `~/.antigravity_cockpit` / `~/.antigravity_cockpit_dev` 建立新名称的兼容入口（Unix 符号链接 / Windows 目录联接），无需手动迁移。旧物理目录和已有实例路径保留，两入口读写同一数据；当前会话保持原路径，后续启动使用新入口。无法建立链接时继续使用旧目录；新旧均为独立目录时保留旧目录，不自动合并或覆盖
  - 受管实例默认位于公共目录的 `instances/<平台>`；Windows 部分平台的实例位于 `%APPDATA%\.cockpit_tools\instances\<平台>`，其旧目录同样自动兼容；自定义目录保持原样
  - `~/.codex`：Codex 官方当前登录 `auth.json`
  - `~/.grok`：Grok CLI 官方默认实例与当前登录 `auth.json`
  - `~/.zcode/v2`：ZCode 官方当前登录加密凭据与配额缓存
  - 系统应用数据目录下 `com.jlcodes.cockpit-tools`：宿主应用的 WebView 状态等；历史 `com.antigravity.cockpit-tools` 目录仅用于旧数据兼容导入
- **Grok CLI 凭据不加密**：access token/refresh token 以明文 JSON 保存在本机，主要依赖操作系统账号隔离与本地文件权限保护；Unix 系统上凭据目录设为 `0700`、凭据文件设为 `0600`。脱敏导出不包含 token，不能作为登录备份
- **WebSocket 默认仅本机访问**：监听 `127.0.0.1`，默认端口 `19528`，可在设置中关闭或改端口。
- **什么时候会联网**：OAuth 登录、Token 刷新、配额查询、版本更新检查等官方接口请求。
- **macOS 隐私权限弹窗说明**：在 Cockpit Tools 中启动 Codex/agent 后，如果 agent 执行的 shell 命令访问桌面、文稿、下载、照片等受保护目录，macOS 可能会把权限请求显示为“Cockpit Tools 想要访问……”。这是因为这些命令是 Cockpit Tools 启动的子进程，系统会把权限归因到宿主应用；这不等同于 Cockpit Tools 主程序主动扫描这些目录。是否允许取决于你是否信任当前 agent 任务和它将要执行的命令；不确定时可以选择拒绝，或先把项目放在普通工作目录中运行。
- **实用安全建议**：
  1. 不使用插件联动时，可关闭 WebSocket 服务。
  2. 不要把用户目录直接打包分享；备份前注意脱敏 token 文件。
  3. 在公共或共用电脑上，使用后删除账号并退出应用。

### WebSocket 账号切换 API

`request.switch_account` 支持明确选择 Antigravity 客户端。未传 `runtime_target`，或传 `"antigravity_ide"`，保持原有 IDE 切号流程及现有启动/无感双通道设置；不会读取前端选择的客户端。

切换 Antigravity 桌面版及使用同一系统凭据的 `agy` CLI，可发送：

```json
{"type":"request.switch_account","payload":{"account_id":"<account-id>","request_id":"x","runtime_target":"antigravity","launch_on_switch":false}}
```

- `runtime_target: "antigravity"`：复用桌面版切号认证规则；2.0 及以上版本写入系统凭据，并同步桌面版的本地回退文件 `~/.gemini/jetski-standalone-oauth-token`（保留 `id_token`，Unix 权限为 `0600`）；旧版写入本地 SQLite 登录状态。只有真实凭据及回退文件写入成功后才更新当前账号、默认实例绑定并通知切号结果。回退文件同步失败会明确报告系统凭据已写入并提示重试。
- `launch_on_switch`：仅用于 `"antigravity"`，默认 `false`；不关闭或启动客户端，已运行桌面版可能需下次启动才使用新账号。显式传 `true` 才执行与桌面版 UI 相同的关闭、写入、重启流程。IDE 请求不要传此字段，继续使用现有设置。
- `request_id`：可选，传入后返回对应的 `response.success` 或 `response.error`。不支持的目标、凭据写入失败或并发桌面切号会返回错误，不报告成功；凭据已切换但启动失败会明确报告启动错误。无效目标返回 `WS_SWITCH_TARGET_INVALID`，在 IDE 请求中传启动策略返回 `WS_SWITCH_LAUNCH_POLICY_INVALID`；并发桌面切号返回 `ANTIGRAVITY_SWITCH_BUSY`，账号准备超时返回 `ANTIGRAVITY_SWITCH_PREPARATION_TIMEOUT`。
- `ANTIGRAVITY_SWITCH_STATE_SAVE_FAILED` 表示客户端凭据已切换，但 Tools 当前账号状态保存失败，请重试切号。默认实例绑定更新保留原有尽力保存行为，失败会记录日志。

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
*   **Windows**: `.msi`（推荐）、`.exe`，或 `x64-portable.zip`（解压即用，无需安装）

Windows 便携包解压后直接运行其中的 `Cockpit Tools.exe`（或同名主程序）。系统需要 Windows 10/11 的 Microsoft Edge WebView2 Runtime；账号和配置仍按当前 Windows 用户目录保存，不会随 ZIP 文件夹自动迁移。
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
| <img src="docs/images/qq_group_20260404_183718.png" alt="QQ 群二维码" width="200" /> | <img src="docs/images/wechat_info.jpg" alt="微信联系二维码" width="200" /> |

---

## ☕ 赞助项目

如果不介意，请 [☕ 赞赏支持一下](docs/DONATE.md)

您的每一份支持都是对开源项目最大的鼓励！无论金额大小，都代表着您对这个项目的认可。

---

## 致谢

- Antigravity 账号切号逻辑参考：[Antigravity-Manager](https://github.com/lbjlaq/Antigravity-Manager)
- Codex API 服务集成 CLIProxyAPI，Codex Live WebRTC/sideband、Responses WebSocket 状态安全、canonical token accounting v2、Multi-Agent V2 兼容、Grok CLI 账号与 OAuth、Grok `apply_patch` 协议兼容，以及账号池错误分类与状态恢复边界参考其开源实现：[router-for-me/CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI)（MIT）
- Codex、Claude CLI 与 Claude Desktop Gateway 第三方供应商预设、模型映射，以及从会话 JSONL 汇总真实用量的方向参考：[CC Switch](https://github.com/farion1231/cc-switch)
- Claude 可选登录 helper 运行时基于：[Electron](https://github.com/electron/electron)
- Codex 账号代理使用 [Mihomo](https://github.com/MetaCubeX/mihomo) 独立进程处理受支持的 Clash 节点与分组，节点参数、嵌套分组与内置阻断策略的配置语义、本地连接日志接口以及健康检查与故障转移设计参考其官方源码和文档；内核由用户主动下载或导入，宿主安装包不内置，不代表官方合作关系。

感谢以上项目作者的开源贡献！如果这些项目对你有帮助，也请给他们点个 ⭐ Star 支持一下！

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
