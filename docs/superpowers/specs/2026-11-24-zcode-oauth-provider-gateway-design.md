# ZCode OAuth Provider Gateway 设计

日期：2026-11-24

## 意图

在已合入上游 `main` 的 ZCode API Key Provider Gateway 基础上，让用户可以直接在 Codex 中使用 Cockpit ZCode 面板已经登录的 OAuth 账号和对应配额。登录、Token 交换、加密存储仍由 ZCode 面板负责；Codex 侧只通过账号引用在请求时解析凭据。

## 目标

- 支持 `ZcodeAuthMode::Oauth` 账号绑定为 Codex Provider Gateway 账号。
- 请求时读取 Cockpit 已保存的 ZCode OAuth 账号，不把 token 写入 Codex `auth.json`、`config.toml` 或 legacy sidecar。
- OAuth 推理仅接入已确认活跃的 Z.ai 个人 Start Plan。BigModel 推理需要项目 API Key。
- 复用现有 Native Rust Responses → Anthropic Messages 转换和流式处理。
- 支持 ZCode OAuth 账号的模型目录和稳定的请求 owner 隔离。
- OAuth 凭据缺失或失效时 fail closed，并提示用户回到 ZCode 面板重新登录。

## 非目标

- 不重复实现 ZCode OAuth 登录、回调和 Token 交换。
- 不修改已经合入 `main` 的 ZCode API Key 行为。
- 不把 OAuth token 投影到 Codex 配置文件或 sidecar。
- 不在本 PR 实现 WorkBuddy。
- 不承诺通过不受验证的 token 组合访问任意 ZCode API；每种计划使用已验证的请求契约。

## 约束与现状

- Cockpit 的 `ZcodeAccount` 已保存加密还原后的 `access_token`、可选 `refresh_token`、`zcode_jwt_token`、provider、team plan 和账号元数据。
- 现有 ZCode OAuth 代码已经完成登录和 token envelope 解析；配额请求使用 `zcode_jwt_token`，BigModel team 请求还使用 `access_token`、组织和项目头。
- 现有 ZCode Provider Gateway 仅接受 API Key 账号，并且 Native adapter 仅生成 `x-api-key`。
- ZCode OAuth 路由需要区分 Coding Plan 项目 key、Start Plan session token 和 Team Plan，并通过 Anthropic Messages API 发送。当前接入复用 ZCode 客户端身份头。
- 该 PR 必须从上游已经包含 `6fb8221f` 的最新 `main` 建立，而不是从旧的 `782b1cb` 分支直接继续。

## 方案比较

### 方案 A：Native Rust Gateway 原生 OAuth adapter（采用）

在现有 `native_zcode` 中增加 OAuth 凭据解析和请求头构造。Codex 账号保存来源 ZCode 账号 ID，运行时读取来源账号并根据计划构造 Anthropic 请求。优点是账号、配额、凭据生命周期都留在 Cockpit 内，Codex 和其他客户端可以复用同一 Provider Gateway；缺点是必须准确实现 ZCode 的计划选择和客户端 headers。

### 方案 B：依赖本地外部 provider 进程

Cockpit 把外部 provider 进程当作独立服务，自己只保存服务地址。实现量较小，但增加外部进程依赖，账号和配额管理分裂，无法满足通用 Provider Gateway 的架构目标，因此不采用。

### 方案 C：把 OAuth token 写入 Codex 配置

实现最直接，但会复制敏感凭据、绕过 ZCode 账号生命周期，也无法表达 Coding Plan、Team Plan 和 Start Plan 的不同认证契约，明确不采用。

### 已确认的产品决策

- 同一账号同时存在个人 Coding Plan 与 Team Plan 时，跟随 ZCode 面板当前选择；Gateway 不自行改写或切换计划。
- 本 PR 不由 Gateway 使用 `refresh_token` 做刷新；凭据失效时 fail closed，并提示用户回到 ZCode 面板重新登录。这样避免在未确认刷新接口的情况下引入 token 并发和持久化风险。
- Z.ai OAuth 需要活跃的个人 Start Plan。BigModel Coding / Team Plan 可通过单独的项目 API Key 账号接入。


新增 OAuth 专用 provider kind，例如 `zcode_oauth`。绑定时检查来源账号、套餐记录和会话 JWT；生成稳定的 Codex provider account ID，并继续将 `openai_api_key` 保持为空。

### 请求时解析

`native_account_credentials` 根据 Codex provider account 的外部引用加载最新 `ZcodeAccount`：

1. BigModel OAuth 在绑定、网关预检和请求前均返回需要项目 API Key 的错误。
2. 若账号走个人 OAuth session/Start Plan，使用 `zcode_jwt_token` 和 ZCode 客户端 headers；失效时返回重新登录提示，不把失效 token 写入其他存储。
3. Coding Plan 项目 `zcode-api-key` 的读取契约尚未在 Cockpit 中安全复用；本 PR 不把 `access_token` 猜测为 API key，也不在无法确认时发出个人 Coding Plan 请求，后续单独实现。
4. 所有敏感 header 设置为 sensitive；owner fingerprint 只包含来源账号和非敏感 scope，不包含 token。

项目 API Key 的读取优先复用现有 ZCode credential/config 解析能力；不能读取或无法确认计划时不得猜测并发送请求。

### 请求协议

继续使用现有 Anthropic wire adapter。OAuth 路径补齐已验证的客户端身份字段、ZCode client title/version 和计划路径需要的 authorization header。每个用户 turn 的 query/request ID，以及 Start Plan 特殊 system block 和风控错误细分，待现有请求上下文 fixture 完整后单独启用。

### 模型与错误

优先读取 ZCode 当前模型目录；保留已有默认模型作为无目录时的安全 fallback。将凭据失效、计划不可用、风控/异常活动、并发限制映射为现有 Provider Gateway 可识别的本地错误，不自动切换到 API Key 或其他账号，避免意外消耗不同配额。

### Sidecar 与安全

OAuth 外部引用只允许 Native Rust Gateway。legacy sidecar 配置继续 fail closed。账号删除、来源账号失效和 token 过期都必须在请求前被识别，不生成包含 token 的 Codex 文件。

## 验证

- 单元测试：OAuth 账号引用、计划路径选择、headers 敏感标记、owner 隔离、失效账号 fail closed。
- 集成级 fixture：Coding Plan、Team Plan、Start Plan 的 Anthropic 请求头和模型路由。
- 回归：现有 TypeScript 测试、release 测试和可用的 Rust 核心测试。
- 真实网络验证只在用户明确提供可用测试账号且不记录 token 的情况下进行；默认不在测试日志中输出凭据。

## 留给实现者的决定

- 复用仓库已有的 ZCode credential/config 解析函数，而不是复制另一套外部 provider 的存储读取代码。
- 若当前仓库无法安全确认 Coding Plan 项目 key 的读取契约，先实现 OAuth session/Start Plan 的 fail-closed adapter，并将 Coding Plan 项目 key 单独列为后续小 PR；不得用 `access_token` 猜测替代 API key。
- 继承现有 Native Gateway 的模型目录和错误结构，避免新建第二套客户端协议。
