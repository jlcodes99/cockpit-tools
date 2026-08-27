# CodeBuddy 反代 API 中转 —— 推理协议规格笔记

> 本文档记录腾讯 CodeBuddy 订阅的推理后端协议，用于指导 cockpit-tools 中「CodeBuddy 反代 API 中转」功能的实现（仿照已有 Codex API Service 三层架构）。
>
> 信息来源：一手源码引用（项目内 Rust OAuth 模块 + 第三方开源项目 `ShouZhuo0413/codebuddy2api`、`emailck/codebuddy2api`）+ 真实中国站 credit 账号端到端实测（2026-08-17）。

## 一、核心结论（TL;DR）

腾讯 CodeBuddy 的推理后端**本身就是 OpenAI Chat Completions 兼容协议**（`/v2/chat/completions`），并非自定义协议。这意味着反代的 translator 工作量大幅降低：

- OpenAI Chat Completions → 几乎直接透传（仅需改 `model`、强制 `stream=true`）
- OpenAI Responses / Anthropic Messages → 转为 Chat Completions（CLIProxyAPI 已有 codex/claude 现成转换器可参考复用）

最难的「认证」环节项目内已完整实现（`codebuddy_oauth.rs` / `codebuddy_cn_oauth.rs`），拿到的就是 `access_token` / `refresh_token` / `uid` / `enterprise_id` / `domain`。

## 二、推理端点

| 站点 | 推理端点 | 证据 |
|---|---|---|
| 中国站 (CN) | `POST https://copilot.tencent.com/v2/chat/completions` | `converter.py` 中 `BACKEND = "https://copilot.tencent.com"`，请求路径 `/v2/chat/completions` |
| 中国站 (CN) 另一路径 | `POST https://www.workbuddy.cn/v2/chat/completions` | **实测确认**：同一 access_token 在两个域名下均返回 200 正常推理 |
| 国际站 (INTL) | `POST https://www.codebuddy.ai/v2/chat/completions`（**未确认，需实测**） | `emailck/codebuddy2api` 的 `CB2PAI_DEFAULT_ENDPOINT` 默认 `https://www.codebuddy.ai` |

> 注意：中国站 OAuth 端点（`www.codebuddy.cn`）与推理端点（`copilot.tencent.com`）**域名不同**。
>
> **实测结论**：CodeBuddy 与 WorkBuddy 是**同一产品的两条调用路径**，调用协议与鉴权法则完全一致。`copilot.tencent.com` 与 `www.workbuddy.cn` 指向同一条推理后端，同一 access_token 在两个域名下均正常推理。账号的 `domain` 字段（`www.codebuddy.cn` / `www.workbuddy.cn`）通过 `X-Domain` header 区分，但推理端点域名可按 `base_url` 任意下发（`copilot.tencent.com` 对 workbuddy.cn 账号同样可用）。

## 三、鉴权 Header

推理请求头：

```
Content-Type: application/json
Accept: application/json
Authorization: Bearer <access_token>
X-User-Id: <uid>
X-Enterprise-Id: <enterprise_id>   # 可为空（credit 账号），空时省略
X-Tenant-Id: <enterprise_id>       # 与 X-Enterprise-Id 同值，可为空
X-Domain: <domain>                 # 如 www.codebuddy.cn / www.workbuddy.cn
X-Product: SaaS
X-IDE-Name: CodeBuddyIDE
X-Requested-With: XMLHttpRequest
User-Agent: CodeBuddyIDE
```

**credit 账号边界（实测）**：无 enterprise 订阅的 credit 账号 `enterprise_id` 为空，`X-Enterprise-Id` / `X-Tenant-Id` 省略后后端仍返回 200。最小可用鉴权为 `Authorization` + `X-User-Id` + `X-Domain`。

## 四、请求体（标准 OpenAI Chat Completions）

`POST /v2/chat/completions` 的请求体是**标准 OpenAI Chat Completions JSON**，透传白名单字段：

```
model, messages, tools, tool_choice, temperature, max_tokens,
max_completion_tokens, top_p, stream, stream_options, stop,
presence_penalty, frequency_penalty, n, response_format, seed,
user, reasoning_effort, verbosity, reasoning_summary
```

关键约束：

1. **强制 `stream=true`**：腾讯后端只支持流式，非流式请求返回 400 `Non-stream chat request is currently not supported`；非流式由反代层消费 SSE 后聚合。
2. 默认 `model="auto"`（客户端未传时，自动路由到实际模型）。
3. 默认补 `stream_options: {"include_usage": true}`。
4. 支持原生 `tools` / `tool_calls` / 多轮工具调用。

## 五、SSE 流式响应

标准 OpenAI Chat Completions SSE 格式（`data: {...}\n\n`）：

- 增量内容：`choices[].delta.content`
- 工具调用：`choices[].delta.tool_calls[]`（含 `id`/`name`/`arguments` 增量）
- 结束：`choices[].finish_reason`
- 用量：`usage`（配合 `stream_options.include_usage`）
- **credit 计费字段**：`usage.credit`（如 `"credit": 0.01`），是 CodeBuddy credit 计费的权威来源，反代层已透传
- 响应 media type：`text/event-stream`

## 六、Token 刷新

| 站点 | 刷新端点 |
|---|---|
| 国际站 | `POST https://www.codebuddy.ai/v2/plugin/auth/token/refresh` |
| 中国站 | `POST https://copilot.tencent.com/v2/plugin/auth/token/refresh`（`www.workbuddy.cn` 同路径亦可，实测） |

刷新请求头：

```
Authorization: Bearer <access_token>
X-Refresh-Token: <refresh_token>
X-Domain: <domain>
```

刷新请求体：`{}`。刷新响应：`{code: 0, data: {accessToken, refreshToken, expiresAt, ...}}`。

## 七、凭据来源

第三方 `codebuddy2api` 直接读桌面端登录态文件 `CodeBuddyExtension/Data/Public/auth/*.info`。

**本项目无需走此路径**：已有 `codebuddy_oauth.rs`（国际站）与 `codebuddy_cn_oauth.rs`（中国站）完成 OAuth 登录，账号数据存于 `CodebuddyAccount`，字段含 `access_token` / `refresh_token` / `uid` / `enterprise_id` / `domain` / `plan_type` 等。反代编排层直接复用这些账号作为凭据来源，注入 sidecar 的 `auths/` 目录。

## 八、协议转换矩阵

| 客户端协议（本地 /v1） | 腾讯后端 | 转换方式 |
|---|---|---|
| OpenAI Chat Completions | `/v2/chat/completions` | 几乎透传：改 model、强制 stream=true、补 stream_options |
| OpenAI Responses | `/v2/chat/completions` | Responses → Chat |
| Anthropic Messages | `/v2/chat/completions` | Messages → Chat |

## 九、内容审核（重要风险）

腾讯后端在推理前会做内容审核，当客户端注入的 system prompt 命中审核时会拦截。translator 需预留脱敏/重试策略入口。

## 十、工具调用 ID 前缀（实测修正）

后端返回的工具调用 ID 前缀为 **`call_`**（如 `call_00_QbmbidktVHZ1ig2CnUnX0328`），**不是**早期逆向的 `tooluse_`。executor 的 `normalizeToolCallID` 已幂等兼容：`tooluse_` → `call_`，已是 `call_` 则原样返回。

## 十一、Go sidecar 扩展点速查

CLIProxyAPI（`sidecars/cockpit-cliproxy/cdk/CLIProxyAPI/`）的 provider 扩展约定：

- **Auth**：auth 文件平铺在 `auths/`，JSON 含 `type` 字段作为 Provider。
- **Executor**：`sdk/cliproxy/service.go` 的 `ensureExecutorsForAuthWithMode`（switch `a.Provider`）与 `registerModelsForAuth`。
- **Translator**：`sdk/translator/registry.go` 的 `Register(from, to Format, ...)`。
- **模型注册**：`cliproxy.GlobalModelRegistry().RegisterClient(a.ID, providerKey, models)`。

## 十二、端到端实测修复记录（2026-08-17）

实测中定位并修复了 Go sidecar 三处 provider 硬编码导致的 codebuddy 路由失效：

1. `readManifestCodexTokenAuth`：auth 文件 `type` 白名单仅 `codex` → 增加 `codebuddy`，且 `Provider` 字段动态化（原硬编码 `"codex"`）。
2. `registerManifestModelsForAuth`：模型注册硬编码 `"codex"` provider → 改为 `auth.Provider` 动态化。
3. `handleNonStream` / `handleStream`：请求 providers 硬编码 `[]string{"codex"}` → 新增 `resolveRequestProviders(model)` 按模型动态解析。

同时修复 Rust 编排层 `manifest_api_keys` 字段名 bug：原生成 `name`/`key`，与 Go 侧 `apiKeySpec`（期望 `id`/`label`/`key`/`enabled`/`accountIds`）不匹配 → 已对齐。

## 十三、图片能力（视觉理解 + 生图，实测校准 2026-08-19）

用真实中国站账号（`docs/codebuddy_cn_accounts_2026-08-19.json`，`domain=www.workbuddy.cn`，`payment_type=free`）实测校准了 CodeBuddy 图片能力的两条链路：

### 13.1 视觉理解（图片输入，已打通）

- 后端 `/v2/chat/completions` 支持视觉理解，但**严格校验图片格式**：`image_url` 必须是对象且 `url` 必须是字符串（`{"type":"image_url","image_url":{"url":"<str>"}}`）。
- 实测非标准格式全部返回 400：
  - `image_url` 为字符串 → `cannot unmarshal string into v2.ImageContent`
  - `url` 嵌套对象（用户反馈的"多一对花括号"）→ `cannot unmarshal object into ... url of type str`
  - `image_url` 为 JSON 字符串（双重编码）→ 同上
- **修复**：`codebuddy_executor.go` 新增 `normalizeCodebuddyChatImageContent`，在翻译后、上送前把 `messages[].content[]` 的图片 part 归一化为标准格式，容忍字符串 / 嵌套对象 / 双重编码 / 内嵌 `image_url` 等多种形态（单元测试 `codebuddy_executor_image_test.go` 覆盖）。这是第三方客户端（Cursor）图片输入 400 的根因修复。

### 13.2 生图（图片生成，协议已确认，受账号权限限制）

- **后端专用端点已确认**：`POST /v2/images/generations` 与 `/v2/images/edits` 是 **OpenAI Images API 兼容**端点（接受 `model`/`prompt`/`n`/`size`/`response_format`），**不是** Chat Completions 的 `image_generation` 工具（实测该工具被后端静默忽略并回复 SVG 文本）。
- 实测关键证据：
  - `tool_choice` 只接受**字符串**（`"auto"`/`"required"`），对象形式 `{"type":"image_generation"}` 返回 400 `cannot unmarshal object into ... tool_choice of type string`。
  - `/v2/images/generations` 对任意模型名返回 `Image model [xxx] route config not found`（code 14401）——**free 账号无生图权限/路由**。
- **协议修正**：codebuddy 生图分支从「Chat Completions + 工具」改为**直接透传 OpenAI Images API 请求到 `/v2/images/generations`**（`main.go` `handleCodebuddyImagesRelay` + `codebuddy_executor.go` `executeOpenAIImage`，非流式）。付费/有图片路由的账号即可用，free 账号会如实返回后端的 400 错误。
- 占位图片模型 ID：`codebuddy-image-1`（对应 Codex 的 `gpt-image-2`），待确认有生图权限账号的官方模型 ID 后替换。

### 13.3 灰度开关

- Rust 编排层 `image_generation_mode`：`disabled`（默认）/ `images_only` / `enabled`。
- 默认 `disabled`：图片模型不进入 `/v1/models`，图片请求被拒绝（模型不可见）。
- `enabled`：图片模型可见，图片请求放行；`images_only`：仅图片请求放行。
- `max_concurrent_image_requests`：单账号图片并发上限（默认 1，上限 16）。

### 13.4 Go 侧路由

- `main.go` `handleImagesRelayRequest` 按 `resolveRequestProviders(requestedModel)` 动态路由：codebuddy 图片模型走 `handleCodebuddyImagesRelay`（`/v2/images/generations` 透传），其余走 Codex 流式。
- `buildImageTool` 支持 `codebuddy-image-1`。
- 图片模型注入：`registry.WithCodebuddyBuiltins`（仿 `WithCodexBuiltins`），静态 `models.json` 同步登记 `codebuddy-image-1`。

### 13.5 测试脚本

- `scripts/diag_codebuddy_vision.py`：脱敏诊断（token 有效性 / 视觉格式矩阵 / 生图探测 / 端点探测），`--only token|vision|gen|endpoints`。
- `scripts/test_codebuddy_images.py`：`--mock` 内置 mock 后端自检；默认模式端到端测试（`/v1/models`、`/v1/images/generations` b64/url/stream、`/v1/images/edits` JSON/multipart、非法模型拒绝、**视觉理解格式矩阵** `test_vision_matrix`）。

### 13.6 待办

1. 用**有生图权限**的账号实测 `/v2/images/generations` 成功响应，确认响应字段（`data[].b64_json`/`url`）与模型 ID。
2. 替换占位图片模型 ID `codebuddy-image-1` 为官方 ID。
3. 若需支持 multipart edits，补充 codebuddy `/v2/images/edits` 的 multipart 转 JSON 逻辑（当前 codebuddy 仅支持 JSON edits）。
