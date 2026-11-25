# 平台账号接入 Codex Provider Gateway

Cockpit 的 Provider Gateway 可以把部分平台账号作为本地、可复用的上游来源提供给 Codex 等客户端。

## ZCode 支持范围

当前 Native Rust 网关支持 **ZCode API Key 和 OAuth 账号**：

- ZCode 账号仍保存在加密的 `zcode_accounts` 存储中；
- Codex 只保存 `apiCredentialKind` 与 `apiCredentialRef`，不保存 API Key 或 OAuth Token；
- 请求发送前，Native Rust 网关按引用读取 ZCode 账号，在内存中生成认证头；
- API Key 账号使用 Z.ai/BigModel Anthropic endpoint 与 `x-api-key`；
- OAuth 个人账号使用 ZCode Plan Anthropic endpoint 与 ZCode session JWT；
- BigModel OAuth Team Plan 使用已有的组织、项目和访问 token 信息；
- OAuth 计划选择跟随 ZCode 面板当前账号选择，Gateway 不自行切换计划；
- Gateway 不自行使用 `refresh_token` 刷新凭据。失效时 fail closed，并提示用户回到 ZCode 面板重新登录；
- 使用已有的 Responses → Anthropic Messages 转换和流式适配；
- 若选择旧 sidecar 网关，会显式拒绝该账号，不会把凭据或外部引用写入 sidecar 配置。

ZCode OAuth 的请求头和 Plan 路由遵循 ZCode 客户端契约；Cockpit 不依赖外部 provider 进程，也不复制其他工具的账号存储。

## WorkBuddy

WorkBuddy 尚未在本模块中接入；后续需要单独实现其认证、模型、流式事件和错误分类适配器，并继续保持本地回环网关默认值。

## 使用方式

在 ZCode 账号页的 API Key 或 OAuth 账号卡片上选择「接入 Codex 网关」。这会创建一个可删除、可独立配置模型目录的 Codex 供应商账号；重复操作会复用同一条绑定记录。
