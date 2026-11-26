# WorkBuddy Provider Gateway 设计

## 目标

在 ZCode OAuth PR 独立合并后，为 WorkBuddy 增加 Native Rust Provider Gateway 适配，使 Codex 等客户端可以通过本地 Gateway 使用现有 WorkBuddy 账号，而不复制凭据或依赖外部桥接进程。

## 已有能力

WorkBuddy 账号已经保存 `access_token`、`uid`、企业信息和域信息。现有模型探测已经实现会话创建、会话地址和 token 获取、ACP JSON-RPC 初始化、模型选择、prompt，以及 ACP SSE 文本和错误解析。

## MVP 范围

第一阶段仅实现 **Native-only、`workbuddy_acp`、无状态、纯文本**链路：

- 支持纯文本输入，读完整条回复后通过 Responses SSE 返回文本；
- 不支持工具调用、图片、reasoning、多轮持久会话或跨请求上下文；
- 不支持 legacy sidecar、自动 failover 或将 WorkBuddy 当作静态 HTTP 上游；
- 只在显式选择 WorkBuddy ACP provider 时执行，不改变其他 provider 路由。

## 采用方案

采用 Native Rust 专用 ACP executor，而不是扩展现有 `wire_api`。现有 HTTP executor 假设一次 POST 对应一次 JSON/SSE 上游响应，无法表达 WorkBuddy 的多阶段 session 生命周期，因此用专用上游地址 `workbuddy-acp://native` 作为路由标记，在进入通用 HTTP 上游分支前拦截并走独立的 `workbuddy_acp` 请求生命周期。

每个请求的流程：

1. Gateway 根据外部账号引用读取最新 WorkBuddy 账号。
2. 校验当前 `access_token`，仅在内存中构造 WorkBuddy billing headers；Gateway 不把 token 写入 Codex 配置、sidecar、日志或会话状态。
3. 创建临时 WorkBuddy conversation，获取 ACP session 地址、session token 和 session ID。
4. 执行 ACP `initialize`、`session/load`、`session/set_model`、`session/prompt`。
5. 将客户端纯文本输入转换为 ACP prompt，汇总 ACP 返回的文本，再发送 Responses 的创建、文本和完成事件。
6. 请求完成、失败或取消后，默认删除临时 conversation；清理失败只保留脱敏诊断，不覆盖原始响应。

## 会话与并发

第一版不保存客户端会话到 WorkBuddy session 的映射。每个 Gateway 请求创建并删除自己独立的临时会话，请求之间没有共享会话状态，因此并发请求不会互相覆盖。持久多轮会话、恢复和跨请求上下文不在本 MVP 范围内。

## 凭据与错误边界

- 使用账号当前 access token；不在 Gateway 内实现 refresh token 刷新。
- 空 token、失效 token 或 session 初始化失败时 fail closed。
- 认证失败映射为本地重新授权错误；限流映射为 429；ACP 超时映射为 504；协议或上游响应无效映射为 502。
- 所有错误和日志必须脱敏，不包含 access token、session token、完整请求内容或会话地址中的敏感参数。
- WorkBuddy ACP provider 不得自动降级到 sidecar、其他账号或其他 provider。

## 验证

使用本地 fixture server 覆盖：

- conversation 创建、session 信息获取和删除；
- ACP JSON-RPC 初始化、模型选择和 prompt；
- SSE 多段文本、完成事件、错误事件和限流事件；
- 请求取消、超时、清理失败和账号级重复并发；
- token 缺失时 fail closed；
- WorkBuddy 账号引用不会写入 Codex 配置或 sidecar；
- 非文本输入、工具请求和图片请求被明确拒绝，而不是静默降级。

## 后续范围

工具调用、图片、reasoning、持久会话、上下文恢复和更完整的客户端语义另行设计；只有在 ACP fixture 和 MVP 线上行为稳定后再扩展。
