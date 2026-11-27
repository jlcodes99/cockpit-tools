# 为什么本修复保留客户端 0/0 重试

本次网关修复不把受管 Codex provider 的 `request_max_retries` / `stream_max_retries` 从 `0/0` 改成混合值。客户端的流重试可以重新发送请求，但在下述准确客户端中，它也会重新发送网关已经明确结束恢复的终态请求；单改两个计数无法区分这两种情况。

## 已实测的客户端与矩阵

2026-11-27，以独立空 HOME 和环回 HTTP 服务运行 9 个合成场景：

- CLI：`codex-cli 0.171.0-alpha.13.2`。
- CLI 文件 SHA256：`2a1ec3f1dd463fd30224be4c0c67d3fe83d3d863a3d7539ccad366952436d2aa`。
- 这是上述二进制的实测，不能仅凭相同版本字符串认定另一个二进制行为相同，也不代表桌面端会热加载 provider 配置。

|合成响应|request / stream|POST 次数|最终结果|
|---|---:|---:|---|
|HTTP 504，`network_recovery_exhausted`，`x-should-retry: false`，正文 `retryable: false`|0 / 0|1|`turn.failed`|
|同上|0 / 1|2|重连后 `turn.failed`|
|同上|1 / 0|2|`turn.failed`|
|HTTP 200 SSE `response.failed`，相同错误码与 `retryable: false`|0 / 0|1|`turn.failed`|
|同上|0 / 1|2|重连后 `turn.failed`|
|同上|1 / 0|1|`turn.failed`|
|SSE 已发送 created/文本 delta 后断开；第二 POST 返回完整成功|0 / 0|1|`turn.failed`|
|同上|0 / 1|2|`turn.completed`，输出 `Synthetic complete.`|
|同上|1 / 0|1|`turn.failed`|

全部 9 个场景完成，无启动错误或场景 watchdog 超时；8 个预期失败场景的 CLI 退出码为 1，合成成功场景为 0。9 个子进程及环回监听均退出，随后进行了独立 PID/端口核验。没有使用真实账号、修改生产配置或发送真实模型任务。

## 不能从成功场景推导的结论

断流 fixture 确实发送了部分文本 delta，但 CLI JSON 输出只展示完整 item；实测回执中 `partialVisible=false`。因此，这不是“用户已经看到部分内容后，原执行无损续接”的验证。

所有第二次 POST 的 `previous_response_id` 均为空。唯一成功场景证明客户端重新发起一次请求后收到了合成完整结果，不证明它恢复了同一上游执行，也不证明真实工具调用或生图不会重复执行。不能把开启 stream retry 描述为普遍安全的恢复方案。

网关内部预算和客户端重试是两层控制。网关已经给出恢复终态后，客户端再次 POST 会启动一个新的网关请求；这可能再次消耗预算并延长用户看到的等待。故本 PR 保留 0/0，并把改进限定于网关的大请求上传计时及压缩内存使用。真正区分终态与可恢复断流，需要客户端理解明确错误语义及有界恢复协议，而不是伪装 HTTP 状态或额度错误。

## 可复跑脚本

运行 `scripts/tests/verify-client-retry-policy.cjs`，两个路径参数都必填且必须是绝对路径：

```powershell
node scripts/tests/verify-client-retry-policy.cjs --codex 'C:\Tools\Codex\codex.exe' --output 'C:\Temp\codex-retry-verification'
```

示例路径是占位目录，应换成要验证的 CLI 和专用输出目录。输出目录必须不存在或为空；脚本拒绝符号链接、目录联接、Windows reparse 路径及文件系统根目录，不覆盖已有测试。每次在该目录内创建独占的运行子目录。

脚本保留上述 3 类 fixture × 3 组重试值，记录 CLI 版本/hash、POST 次数、`previousResponseId`、最终事件、`partialVisible`、退出和清理状态。它用独立 HOME/USERPROFILE/CODEX_HOME/APPDATA/LOCALAPPDATA/TEMP、环境白名单、`--strict-config --ephemeral --ignore-rules` 与禁用项目文档的配置，provider 只指向动态 `127.0.0.1` 端口。不会读取常用 Codex 配置、认证、聊天或其他项目资料。

此公开 runner 当前限 Windows。每个场景有 25 秒 watchdog，另外限制进程退出等待、200 ms 管道排空和后代清理时长。CLI 退出不等于 stdout/stderr 已关闭：后代进程可能继承管道。脚本等待 CLI `exit` 后有界排空再关闭自己的管道，不永久等待 `close`。

Windows watcher 在 CLI 启动前准备好，之后仅从自己创建的 CLI 出发，以 ParentProcessId 定向查询前瞻登记后代；记录 PID、父 PID、创建时间、Process.StartTime、可用的可执行路径及命令行 hash。正常结束与超时均对已登记进程逐项复核身份后才清理，不按进程名称全局查杀。路径不可读时不会猜路径，其他身份字段仍须一致；身份变化或不可完成清理写入失败回执。若短命进程在枚举与身份读取之间已经退出，会再按准确 PID/创建时间查证，确证消失或换代才记录为采样竞争，不终止那个 PID；仍存活但不能完整核身份继续报失败。只处理自有 ChildProcess、已观测后代、连接、监听和 watcher，不删除输出目录。

回执分别记录 `childExited`、`stdioHeldAfterExit`、`descendantCleanup` 和 `cleanupIntervention`。需要清理残留后代的场景不能说成“自然退出无干预”；管道仍持有却没有已登记后代解释、超时、CLI 从未访问 fixture、缺少预期 turn 终态或 marker/成功输出、版本/参数不兼容或清理不完整均不能当有效场景通过。采样并非操作系统 Job 对象的强制进程隔离，不能承诺在宿主被强杀或未捕获的极短派生链下绝无残留。

原矩阵来自本轮实际执行的隔离 harness。首次公开 runner 复跑只完成前 3 个 HTTP 场景，第 4 个 SSE 场景中 CLI 已退出、遗留的直接子进程仍持有管道，runner 等待 `close` 挂起。该次运行以失败结束，不能冒充 9 场景通过；原本的 25 秒 watchdog 因等待事件选择错误也未及时结束整个场景。

之后补入上述 exit/有界排空及身份严格校验的前瞻后代清理。受控假 CLI 验证创建 detached 后代继承管道、让父进程正常退出：观察到管道仍持有，精确清理登记后代，约 4.9 秒结束，watcher 退出且无登记进程残留。第二次公开运行在第 5 场因短命后代已退出而身份读取竞争被判失败；此记录同样保留，没有把它误计成通过。

最终公开 runner（文件 SHA256 `44ca6e849d3f004a4e3ad1982819f55fb35fd591f512b2a6620ca4cecc581861`）于 2026-11-27 再跑全部 9 场景，runner 退出码 0；独立核对每场 POST 次数、CLI 退出码、预期终态、marker/成功输出和无超时，均与上表一致。两个 SSE 场景（0/0 与 1/0）需要精确清理已观测的自有后代，回执 `cleanupIntervention=true`，不称它们自然退出。全部 watcher 已退出，复查已登记 PID 和 9 个监听端口均无残留。语法与缺参数失败检查也通过。未来客户端版本仍需按新二进制重测，不把本表当永久契约。

输出是本地验证回执，stdout/stderr 可能包含执行路径；对外分享前只选脱敏摘要，不整包公开个人目录或环境信息。
