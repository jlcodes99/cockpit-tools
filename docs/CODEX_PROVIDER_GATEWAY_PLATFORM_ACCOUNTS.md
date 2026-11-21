# 平台账号接入 Codex Provider Gateway

Cockpit 的 Provider Gateway 可以把部分平台账号作为本地、可复用的上游来源提供给 Codex 等客户端。

## 当前支持范围

本变更首个增量只支持 **ZCode API Key 账号**，并且只支持 Native Rust 网关：

- ZCode 账号仍保存在加密的 `zcode_accounts` 存储中；
- Codex 只保存 `apiCredentialKind` 与 `apiCredentialRef`，不保存 API Key；
- 请求发送前，Native Rust 网关按引用读取 ZCode 账号，在内存中生成 `x-api-key`；
- Z.ai 使用 `https://api.z.ai/api/anthropic`，BigModel 使用 `https://open.bigmodel.cn/api/anthropic`；
- 使用已有的 Responses → Anthropic Messages 转换和流式适配；
- 若选择旧 sidecar 网关，会显式拒绝该账号，不会把空凭据或外部引用写入 sidecar 配置。

ZCode OAuth 本次不接入 Provider Gateway：仓库目前没有经过验证的请求时刷新、上游鉴权和流式契约。用户仍可按原有 ZCode 客户端流程使用 OAuth 账号。

## WorkBuddy

WorkBuddy 的现有模型探测流程是会话级 ACP 协议：先创建会话，再获取会话专用地址和令牌，最后读取 ACP SSE 事件。它不是 OpenAI Responses、Chat Completions 或 Anthropic Messages 的静态上游，因此不能安全地复用本次 ZCode 适配器。

后续 WorkBuddy 适配需要单独的会话、模型、流式事件和错误分类适配器，并应继续保持本地回环网关默认值。

## 使用方式

在 ZCode 账号页的 API Key 账号卡片上选择「接入 Codex 网关」。这会创建一个可删除、可独立配置模型目录的 Codex 供应商账号；重复操作会复用同一条绑定记录。
