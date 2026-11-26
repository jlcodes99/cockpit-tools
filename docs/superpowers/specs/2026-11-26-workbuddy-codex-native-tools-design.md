# WorkBuddy → Codex 原生工具桥接设计草案

状态：设计经用户批准；独立实现已通过真实 Codex 0.172 的读文件、失败测试、修复、通过测试和第二用户轮验收。新增预算累计使用 10/20 次真实模型请求。详见[验收记录](../../workbuddy-codex-native-validation.md)。

## 1. 验收目标

不是“能回复文字”，而是实际安装的 Codex CLI 通过 Cockpit API Service，使用指定 WorkBuddy 登录来源完成：

1. Codex 自己读取隔离项目中的文件。
2. WorkBuddy 模型根据实际文件内容提出工具调用。
3. Codex 按自身权限与沙箱规则修改文件、执行命令与测试。
4. 相同 WorkBuddy 会话收到实际工具结果后继续推理并给出正确最终结论。
5. 全程不切换其他账号或上游，不把 WorkBuddy token 写进 Codex 配置，不由 WorkBuddy 的内置文件/终端工具代执行。

只有上述闭环真实通过，才创建独立 feature PR。先前修复 PR #3301 不代表已满足这一目标。

## 2. 已知事实与证据等级

### 已验证

- **原生外部工具实测闭环通过**：真实 WorkBuddy 登录、`glm-5.3-flash` / `low`，注册 `probe_echo`，模型原生发出 `_codebuddy.ai/delegateTool`，ACP prompt 在回包前未完成；测试端返回提示中不存在的随机 challenge，最终 assistant 文字与该 challenge 完全相等，stopReason=end_turn、outcome=SUCCESS。
- 该次只观察到 DelegateTool，没有未知客户端请求。此前一次权限拒绝后明确终止，没有转用其他内置工具；这不是对所有恶意输入/插件旁路的完整安全证明。
- 累计 4 次真实模型 HTTP（包含首次不支持 effort、一次权限拒绝、成功轮的调用前后两次模型推理），另 2 次初始化 HTTP（config、地域/IP检测）。后续 metadata 请求被禁止，使用隔离 profile 的 last-good cache。未超出已批准的 5 次模型请求额度，剩余 1 次暂不使用。
- 当前云端 ACP 文本适配器会拒绝 Codex 的 tools，不能直接用于 coding agent。
- 本机安装的 WorkBuddy CLI 声明 `delegateToolsSupport: true`：已通过无凭据、禁止上游出站的 ACP initialize 实测。
- 启动方式需要沿用项目已有的 `--serve --host 127.0.0.1 --port 0 --auth password --acp --acp-transport stdio`，而非假设单独 `--acp` 即等价。

### 静态确认、仍须实测

- `_codebuddy.ai/delegateToolsChanged` 可注册外部工具定义。
- 模型可调用原生 `DelegateTool { toolId, input, timeout? }`；CLI 用 `_codebuddy.ai/delegateTool` 请求客户端并 await 其结果。
- `--tools DelegateTool` 的 session 白名单会拒绝其他内置工具。
- 各外部工具 schema 写入 DelegateTool 的描述，而不是逐项独立 model function；必须验证实际 schema 遵循情况。
- 回调中的 sessionId 是空字符串，不能作为会话关联键。必须按专属 CLI actor、连接及 JSON-RPC ID 精确关联。
- timeout 参数不是已证明的真正 deadline；取消信号也没有在 delegate manager 中实际消费。

### 尚未证明

- 首次实际注册与 nonce 闭环、Codex 两用户轮的跨 HTTP 读改测均已通过；更长时间的人工审批等待仍需另测。
- 当前安装版本的实际 Responses 互操作已通过，不泛化为所有客户端/模型/CLI 版本。
- 生产生命周期/权限边界已有确定性回归测试；不宣称对恶意本地 CLI 的操作系统级隔离。

## 3. 方案比较

### A. 原生 DelegateTool 桥（优先）

使用现有安装 CLI 的原生外部委托机制，将 Codex 工具注册成外部工具。网关只翻译与关联协议，不执行项目操作。

优点：实际客户端已声明支持；不需从模型文字中猜测工具调用；可只保留一个没有本地执行能力的 DelegateTool。

代价：必须运行受管理的 CLI 子进程；需要已安装且协议兼容的 WorkBuddy CLI；泛化 wrapper 需要严格 schema/白名单校验。

### B. 原生 MCP 桥（备用）

将 Codex 工具通过 CLI 的外部 MCP transport 暴露，等待 tools/call 的真正请求，再交还 Codex 执行。

优点：每个工具可拥有独立标准 MCP schema。

代价：需额外验证 MCP transport/owner/RPC 关联及内置工具禁用；当前证据不足以宣称比 A 更成熟。A 不通过时，再明确决定是否转 B，不悄悄切换。

### C. 聊天文字约定或 WorkBuddy 自己执行（不采用）

从文本扫描 JSON、假装函数调用，或让 WorkBuddy 自己改代码再显示结果，都不能证明 Codex 保有本地工具权限控制，不属于本需求的替代成功标准。

## 4. 建议首版架构

```text
Codex /v1/responses
    │ 已认证的 caller + 固定 WorkBuddy 来源
    ▼
WorkBuddy conversation registry / actor
    │ 专属 CLI / ACP 连接；内置工具仅 DelegateTool
    ▼
WorkBuddy 模型的原生 DelegateTool 调用
    │ _codebuddy.ai/delegateTool(JSON-RPC request)
    ▼
actor 保存 pending RPC，暂停该 await
    │ Responses function_call / custom_tool_call
    ▼
Codex 自己审批、沙箱执行工具
    │ 下次 Responses 请求中的实际 tool output
    ▼
相同 actor 严格验证 → 向原 RPC 回一次结果 → 模型继续
```

### 4.0 实际接入契约（首版）

1. 明确仅支持 **Native Rust API Service**；sidecar 模式提前报不支持，不以隐藏切换替代用户配置。
2. 新建显式 `native_delegate` 执行模式，旧账号/旧路由默认仍为 `cloud_text`。模式在受管 WorkBuddy 专属路由上配置，绑定整个 actor；不根据每次请求有无 tools 决定模式。续轮省略 tools 仍进入原 actor。
3. 提供一个明确的“WorkBuddy → Codex 原生工具桥”配置入口：选择现有 WorkBuddy 来源、从 CLI 当前 catalog 选择模型/允许的 effort，创建或选择 **仅授权该来源的 scoped 本地 API key 与严格模型路由**；不改 collection 主 key 和其他账号池。
4. 为该接入写入独立 Codex profile，使用 scoped 本地 key 和 loopback Native API URL；保留可恢复的 profile backup。现有通用 `__api_service__` 接管会写 collection 主 key，不能直接复用这一行为来冒充来源固定。
5. 实际 profile 强制 HTTP Responses，`supports_websockets=false`；不能仅让 E2E 额外手填关闭 WS，而真实接管配置仍宣称 WS 可用。
6. 公开模型以受管 namespace/真实 model ID 组成（例如 `workbuddy/glm-5.3-flash`），严格映射为 CLI model ID。`workbuddy-default` 只属于旧文字路径，不能暗中把它解释成未经确认的模型。首版默认可选实测过的 `glm-5.3-flash` + `low`，但必须在当前 catalog 中仍存在。
7. 模型/effort 必须来自实时 catalog，映射写入 profile 与网关配置；不支持的 effort 明确拒绝，不静默改成高档位或 CLI 默认模型。运行时 currentModelId 不匹配即失败。
8. 保留 WorkBuddy 直接账号注入/Provider Gateway binding guard；只更新文案以准确说明“编码仅经显式 Native API Service 桥接”，不全局宣称 tool calling 仍不支持。
9. 初版范围为 function/freeform custom/namespace 的本地工具，明确不支持 hosted search、grammar-constrained custom、images、compact、下游 WebSocket。生产独立 profile 显式 `web_search="disabled"`，请求只支持 `tool_choice=auto`；`none`、强制选择及其他未实现的约束在上游前拒绝。parallel_tool_calls=false 串行约束显式执行；并行调用使用有界 pending map，不能静默丢弃。

### 4.1 生命周期

actor 不属于单次 HTTP 请求。正常结束一个工具交接的 Responses 回合，不应终止上游 CLI 或丢弃 pending reply。

状态：Starting → Running → AwaitingClientResults → Resuming → Running → TurnComplete/Ready → Running（下一用户轮）；仅 Cancelled / Expired / Failed / 显式关闭才终止整个会话。

同一个 ACP session 在一次最终答复后保留到 TTL；维护已确认 canonical input cursor，只发送新用户输入/新工具结果。完成一次读改测后还必须通过第二用户轮的上下文延续测试。

首版建议每个逻辑 conversation 一个 CLI，独立 stdout reader、事件缓冲、pending map 和有界队列。不在全局 registry mutex 下 await 子进程或模型。

### 4.2 所有权与回放

每个 actor 固定绑定真实认证 caller、thread/session、WorkBuddy account/source generation、模型和随机 actor generation。public response/call IDs 使用随机值，不暴露凭据或内部 RPC ID。

工具结果必须属于同一 owner、同一来源、同一待执行 call。previous_response_id 不是单独有效的 bearer capability。

**实际 Codex 续轮不能只依赖 previous_response_id**：支持不带该字段、携带完整历史的输入。以已认证 caller/thread + 随机 public call ID 定位暂停 actor，验证已确认历史前缀和新的增量；历史中已消费结果不能再次投递给 RPC。工具交接响应必须接入现有 identity/checkpoint 观察链，不只是临时拼 SSE。若缺少足以安全关联的身份信息，明确失败而非猜测会话。

**不可迁移边界必须在通用 mapping/rebuild 之前建立**：识别 native_delegate 路由/已知 actor 引用，验证来源后 pin owner/account/source/mode。禁止 account-switch rebuild、quota/cooling 重选、capability fallback、网络/stream bootstrap 自动重建，以及 CLI 崩溃后按历史隐式创建新 actor。原 actor 丢失或来源不可用即明确失败；只在 executor 内关闭 retry 不足以满足该约束。

同一结果重试可幂等合并；同 ID 不同结果、跨用户/会话结果、未知或重复结果应明确冲突。结果只回送一次，不重新执行命令。

首版不宣称进程重启后可无损恢复 pending MCP/delegate await；失去原连接时 fail closed，要求新会话，禁止静默重放工具或更换账号。

### 4.3 工具契约

- 工具目录只来自当前 Codex 请求已声明的工具；用稳定内部别名映射原 function/custom/namespace 身份。
- function arguments 校验为相应 schema；禁止网络 schema 引用和无限深度/大小。
- custom 工具的原始字符串独立包装与还原，不改写 patch，不猜测字符串是 JSON。
- 发布调用前完成完整参数校验；文本或工具活动日志不能升级为可执行调用。
- 支持有界 pending 集合；若首版限制串行，应明确排队，不丢弃第二个调用。
- 批次工具目录固定；后续请求不能改变已发布调用的含义。

### 4.4 执行与凭据边界

- WorkBuddy 只允许 DelegateTool，拒绝 fs/terminal/未知 ACP callbacks；插件、MCP、子代理等不能另开执行旁路。
- 不以临时 cwd 冒充安全沙箱。
- Codex 是本地执行者，网关不运行模型 shell 或直接修改项目。
- CLI 使用独立私有 profile，不继承其他供应商 API key、NODE_OPTIONS、自定义 endpoint/header、现有 WorkBuddy 全局配置。
- WorkBuddy token 仅在受控服务/子进程内存中使用，不写入 Codex 配置、命令行、fixture、PR 或日志。不沿用 T3 launcher 将 auth-secret 持久化的做法。
- CLI 本地 HTTP 服务仅绑定 loopback、随机端口且启用密码；不使用无认证模式。
- 必须检查实际 CLI capability；不兼容或未安装时给出明确错误，不能静默转文字或其他账号。

### 4.5 资源与取消

区分 HTTP request permit 与跨回合 actor/account lease。限制全局、caller、账号 actor 数、pending calls、schema/结果/事件字节、模型执行时间、等待审批/工具的 idle timeout 和绝对 TTL。

正常工具交接保留 actor；主动取消、TTL、来源失效、服务关闭则向所有 pending RPC 回明确失败，取消 ACP，随后有界终止自有子进程并清理映射。迟到结果返回明确 expired/conflict，而非重建会话。

### 4.6 Responses 和计费

完整原生调用可产生 output_item.added → arguments delta/done → output_item.done → response.completed，结束的是交接回合，不是伪造整个上游 prompt 完成。

最终上游 usage 可能跨多个 HTTP 工具回合才到达，不能每次重复计费，不能编造 0 token 或 OpenAI quota。保留脱敏 source/actor/model 关联证据。

## 5. 实施范围和顺序（确认后）

1. 建独立 feature 分支/工作树，保留 #3301 及原用户分支；明确依赖关系。
2. 加入可测试的 CLI 发现/安全启动和 ACP reader/writer；内置工具白名单及权限拒绝。
3. 加入 actor registry、owner/source/model 绑定、pending/continuation/cancel/TTL。
4. 加入 Codex tool catalog/原生 delegate/Responses 双向映射及严格错误处理。
5. 接入 WorkBuddy API Service 的 Responses 路由，保留现有纯文本行为；不能删掉 guard 来假装支持。
6. 更新错误文案和使用说明。首版优先实际 Codex CLI 经 API Service 接入；任何直接客户端注入支持必须另有真实配置验证，不默认为已支持。
7. 确定性协议/权限/会话回归测试通过后，再进行受授权的真实 Codex E2E。
8. 审核最终 diff、复跑相关测试、确认无凭据/临时产物提交，然后推送独立 PR。

## 6. 测试矩阵

### 不消耗真实配额

- 完整 function/custom tool 往返，原参数/patch/结果保真。
- namespace/工具别名、未声明工具、错误 schema、超限 payload。
- 跨 caller/thread/account/source generation 拒绝。
- 重复/冲突/乱序结果与并发 continuation。
- 多 pending call、正常 HTTP 完成后等待保留。
- callback 拒绝、超时、取消、CLI 崩溃、service shutdown。
- 无内置 fs/terminal/插件执行旁路；无账户或文字 fallback。
- SSE event ordering、非流式 Responses、真实终止状态与 usage 只记一次。
- 凭据不出现在配置、参数、日志、测试 fixtures、返回 JSON 中。

### 受授权的真实测试

首轮最多 5 次真实模型短请求，仅无害合成工具；已使用 4 次，停止继续消耗。独立记录 2 次不调用模型的初始化 HTTP。请求/重试计数持久累计，不重置预算。

本阶段预算已获用户同意：**新增最多 20 次真实模型短请求**，自动重试/错误请求同样计入，使用同一 WorkBuddy 来源，仅临时合成项目，不上传实际项目。初始化 HTTP 单独限量记录，未知出站默认拒绝。额度用尽即停止并报告，未经确认不扩大。

真实 Codex E2E：仅临时合成项目，先观察失败测试，再实际读取文件/修复/重跑，检查 diff、实际测试退出码和最终答案。然后第二用户轮必须记住前轮结果并完成后续任务；覆盖 full-history continuation。记录实际 WorkBuddy 来源与模型，不使用其他账号 fallback。成功前不提交 feature PR。

## 7. 当前待决事项

- 已解决可行性与编码验收门禁：原生外部工具闭环和真实 Codex 两用户轮 coding 验收均通过。
- 用户已认可设计与首版 Native API Service + 专属 profile 范围。
- 新增最多 20 次模型请求的 E2E 预算已批准（不上传实际项目）。
- #3301 已经由上游 squash 合并；功能 PR 将独立提交，不重复原修复。
