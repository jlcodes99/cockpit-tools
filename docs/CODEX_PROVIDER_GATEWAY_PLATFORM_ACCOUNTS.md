# 平台账号接入 Codex Provider Gateway

Cockpit 的 Provider Gateway 可以把部分平台账号作为本地、可复用的上游来源提供给 Codex 等客户端。

## ZCode 支持范围

当前 Native Rust 网关支持 **ZCode API Key 和 OAuth 账号**：

- ZCode 账号仍保存在加密的 `zcode_accounts` 存储中；
- Codex 只保存 `apiCredentialKind` 与 `apiCredentialRef`，不保存 API Key 或 OAuth Token；
- 请求发送前，Native Rust 网关按引用读取 ZCode 账号，在内存中生成认证头；
- API Key 账号使用 Z.ai/BigModel Anthropic endpoint 与 `x-api-key`；
- Z.ai OAuth 个人账号（Start Plan）使用 ZCode Plan Anthropic endpoint 与 ZCode session JWT；
- BigModel OAuth 账号（Coding / Team Plan）需要项目 API Key，Cockpit 当前未收集该字段，在绑定、网关预检与实际请求三个入口 fail closed，并提示改用 BigModel API Key 账号；
- 绑定和每次推理前都检查已保存的订阅记录，只有活跃套餐明确属于个人 Start Plan 时才使用 OAuth。缺少记录时请在 ZCode 页面刷新额度；
- Gateway 不自行使用 `refresh_token` 刷新凭据。失效时 fail closed，并提示用户回到 ZCode 面板重新登录；
- 使用已有的 Responses → Anthropic Messages 转换和流式适配；
- 若选择旧 sidecar 网关，会显式拒绝该账号，不会把凭据或外部引用写入 sidecar 配置。

ZCode OAuth 的请求头和 Plan 路由遵循 ZCode 客户端契约；Cockpit 不依赖外部 provider 进程，也不复制其他工具的账号存储。

## WorkBuddy

在 WorkBuddy 账号页选择「接入 Codex」，可通过 Native Rust 网关发送独立的纯文本请求。每次请求创建临时会话，读取完整回复后返回给客户端，并尝试删除会话。

## 使用方式

在 ZCode 账号页的 API Key 或 OAuth 账号卡片上选择「接入 Codex 网关」。这会创建一个可删除、可独立配置模型目录的 Codex 供应商账号；重复操作会复用同一条绑定记录。
