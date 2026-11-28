# 原生 Responses 在控制事件后断流：有界同账号恢复

状态：功能补丁与定向验证报告，提交作者审查；未部署，仍有下面明确保留的上传时序验证限制。上游基线为 `a243d9415da9a45a61dc9f98451c615bfad4ffcd`，面向作者仓库 `jlcodes99/cockpit-tools`。本文不包含真实账号、请求 ID、聊天 ID、凭据、视频帧或生产请求正文。

## 问题与这次改动

原生 Codex 的 HTTP Responses 流已经交付 `response.created` 等控制事件、尚未交付业务事件时，如果底层 TLS/HTTP 连接断开，旧代码直接结束这次响应。现有首事件前 bootstrap 恢复已经越过适用窗口，而受管客户端 `request_max_retries=0`、`stream_max_retries=0` 不会替网关再次发送；用户需要手动继续。

本改动给满足严格条件的请求提供一次网关内部恢复：保存首次实际发送的最终请求，保持原账号、完整请求体、身份与预算，刷新失效的传输后最多再发送一次。客户端仍只有一次请求。纯原生命名空间恢复也可适用，额外使用格式转换前的原始流安全证明。

这是**完整相同请求的有限重新采样**，不是原推理的断点续传。它可能产生额外计算或计费。不能保证网络永不掉线，也不覆盖已经输出文本、推理、工具调用或未知执行状态的任意断流。历史失败没有完整流事件，故不能将模拟恢复当作该历史请求已被证明可恢复。

## 现场证据及排除边界

以下是用户授权范围内的脱敏本地诊断统计。它们说明为什么开展这次修复，不作为自动化测试输入。

| 观测 | 实际数据 | 能说明什么及限制 |
| --- | --- | --- |
| 同一主聊天保留记录 | 一致快照442条：371成功、67客户端取消、3读流502、1连接恢复耗尽504 | 有滚动留存，不是全部历史；取消不能混作上游故障 |
| 三次同类读流失败的请求体 | 101,763,036 / 143,457 / 11,094,742字节 | 大小跨度明显；不能断言存在统一大小阈值，也不能排除大输入增加暴露时间 |
| 最新主请求 | first response 19,258ms；total 49,532ms；1 attempt；HTTP/2；502/upstream_stream_error/stream_read | 底层缺少TLS close_notify，不是本地开流/空闲超时分类；首响不等于已输出业务内容 |
| 邻近成功 | 上游请求11,093,859字节，比失败少883字节 | 同等大小可以成功，不证明内容相同 |
| 主+正式关联的3子任务 | 2026-11-28 10:34一致快照中，结束时间自09:17:02起的已存212请求：198成功、12取消、2读流失败；生命周期峰值5并发 | 按captured_at到完成时间计算网关记录重叠，不是模型算力并发，不含未完成/过期/未关联任务；与上行442条历史总体不同 |
| 同次另一子任务失败 | 上游71,468,586字节；first 29,933ms；total 49,190ms；1 attempt HTTP/2 | 比主请求早约11秒，说明问题不只发生在一个窗口；不能据此认定超并发 |
| 当前磁盘设置 | 开流120s、空闲300s；WebSocket关闭；client retries 0/0 | 不是故障时运行时读回；加到600s不能接回已断开的连接 |
| 无线事件 | 主失败前约7秒发生无线安全停止/启动/成功；子任务失败早于这些事件 | 安全会话事件不能直接等同Wi-Fi链路掉线；有关联线索而无因果证明，另一历史失败附近没有对应无线事件 |
| 内存/磁盘 | 现场检查可用内存约9.15/22.77GiB；C盘约95GiB空闲；所查时段无资源耗尽2004 | 没有故障瞬间峰值曲线，不能断言资源从未紧张 |

最早同类错误早于1.5.3发布。固定v1.5.2与v1.5.3的流读取和受管0/0配置相关代码没有发生对应变化：`upstream.rs`及接管策略blob相同，`http.rs`差异为前置native-delegate分流。不能由此断言所有新改动绝不影响时序或概率；没有使用真实账号、相同输入和相同代理做版本A/B。

历史请求payload记录为空，缺少最后业务事件与实际能力声明，因此不猜测图片张数、是否执行了服务端工具或具体断线责任方。也没有证据证明这是全池真实额度耗尽、授权账号掉线、某张图片损坏或官方全站故障。

主任务及三个关联子任务的记录数分别为56/58/61/37。两次失败请求开始时均有4条在途请求（含自身），主请求失败时其他在途2条，子请求失败时其他在途3条；这与整个生命周期里先后重叠过多少请求是不同指标。没有将“抽帧很多”直接等同“发给上游的图片很多”，也没有用CPU任务并发推算模型请求并发。

作者补充“大附件不能要求在很短时间内处理完”，这是合理的容量与计时设计要求。举例中的200M/10秒不是本次测量值；开流、上传进展、流空闲和总预算应分别核实。本次失败已经收到HTTP响应，最终是typed transport `stream_read`错误，不能把延长600秒当作重连实现。下面另行保留24MiB上传计时回归的失败和基线对照，不把它与这次生产TLS故障混为一个原因。

## 安全边界与实现细节

1. 仅原生Codex Responses单个流式HTTP请求的首次上游发送。继续排除官方Plan、混合路由、Chat/Anthropic/Grok/WorkBuddy适配、非流式、compact、图片生成、orphan delegation、opaque turn-state/previous-response与未支持的能力声明。
2. 原始与最终工具声明同时校验。只允许结构有效的客户端function/custom/namespace；拒绝hosted工具、未知类型、畸形字段、服务端prompt引用，以及input内`additional_tools`等尚未证明安全的能力载体。拒绝时保留原请求，不删除工具、帧、历史或图片。
3. 最终请求构造完成后、首次实际发送前捕获已归一化/压缩的`reqwest::Request`。重发使用相同方法、URL、headers和字节体，不重新编码图像、不重新构造历史。捕获锁存不会因diagnostic回滚或已有bootstrap重试重新武装。
4. 控制帧允许范围严格受限；任意output skeleton（即使为空）、text/reasoning/tool arguments、业务/未知/矛盾SSE事件、identity修复、terminal以及错误时未完整原始帧都阻止新恢复。
5. 纯原生命名空间转换只重命名/标记事件，不执行工具。转换前RawProof先观察整批原始帧、原始字节/事件上限和原decoder尾部，再允许转换；仅真实source transport error可作恢复依据。重开继续使用首次冻结Context，防止输出优化后的内部namespace。
6. 失效连接池按已有机制退休；不强制刷新OAuth、不切换账号，不主动取消其他请求。不承诺物理损坏的HTTP/2连接上其他复用流不受传输故障影响。刷新后重新核对account/owner/continuity、endpoint/model/protocol、headers/proxy、授权版本/命名空间和安装身份，不一致则在发送前失败。
7. 额外dispatch直接执行冻结请求，不重入通常的账号/bootstrap/压缩回退循环。body变为不可clone，避免reqwest对该额外发送再做隐式H2重发；第二条流不能重新启用一般pre-business重试。
8. 保留原total/overload截止时间和一次性dispatch限制；本次断流按已有策略启用有界network恢复窗口，成功headers仍按既有机制结束该network episode，不续期总预算或额外发送资格。恢复在当前响应future内等待，客户端取消/服务stop会中断等待并释放permit；没有脱离客户端生命周期的后台重发。
9. 保留真实错误终态。裸EOF、空闲超时、parse错误、已输出工具/文本等不伪装成成功，也不通过假quota错误阻止客户端。

未修改全局超时、受管0/0重试、并发上限、账号池、凭据、代理、视频帧质量或历史。不能用关闭所有重试后的旧保护作为理由忽略可安全恢复窗口，也不能为覆盖更多场景直接打开无限重试。

捕获的请求共享不可变body字节，并非再次复制整份图片，但会把原本上传后可能释放的wire body保留到首个禁止恢复的业务事件或请求结束；并发大输入时这段额外保留仍有内存成本。测试中的大data URL是合成字节，用于核对传输完整性，不是让真实模型识别视频帧，也不能代替真实峰值内存曲线或上游负载实验。

验收发现的栈占用问题已通过将候选状态与恢复future放到堆上处理，未放宽线程栈。x86_64实际测得`Option<Candidate>`为1360字节，`Option<Box<Candidate>>`为8字节，最终Fence为32字节；Responses/compact入口future从本补丁首版27960降至21200字节，未修改官方base为21160字节，最终比base增加40字节。这是具体类型布局，不是整条调用链栈峰值，也不是用户生产断流原因的证明。

## 为什么不直接把客户端stream retry设为1

当前安装CLI的独立loopback实验中，stream retry=1不仅重发模拟断流，也重发合成provider模拟返回的网关`upstream_total_timeout`终态，产生新的POST。结合网关每请求独立预算的实现可推断：该客户端行为接到真实网关时，会启动新请求预算。实验本身并未经过真实网关；第一版隔离还观察到默认MCP连接尝试（被失败代理阻断），所以不能将其称为零旁路测试。

后续隔离预检使用严格配置、空的子进程应用数据目录、文件凭据后端、关闭Apps/plugins/MCP及项目指令、禁用守护进程，并记录本地拒绝代理。实际预检1 POST完成、0代理旁路请求、无MCP错误；它不等于操作系统网络沙箱。随后完成的正式gateway端到端测试验证了双response ID兼容、1个客户端POST与最多2个上游POST以及预算终态不重发，结果如下。

最终修正版的精确已安装CLI `0.171.0-alpha.13.2`（SHA256 `2A1EC3F1DD463FD30224BE4C0C67D3FE83D3D863A3D7539CCAD366952436D2AA`）端到端两项已实际通过：

| 场景 | 客户端POST | 上游POST | 实际客户端结果 |
| --- | ---: | ---: | --- |
| metadata后HTTP body断流、网关恢复 | 1 | 2，body逐字节一致 | exit0，两个不同created ID传到客户端，收到准确唯一答案与turn.completed，turn.failed=false |
| 本次network恢复预算耗尽 | 1 | 1 | exit1，真实network_recovery_exhausted，turn.failed，无完成/答案、无新POST |

两项本地拒绝代理请求均为0，子进程已退出、准确测试入口端口不再监听。客户端使用了最小合成模型的fallback metadata（输出该提示），因此不推断真实Desktop全部模型/插件工具集都满足准入。HTTP断流由这组CLI联测验证，TLS/H2及纯namespace路径由独立测试验证；没有把不同测试场景混为同一个实测。

## 实际验证结果

最终库测试编译成功，构建和运行期间源码指纹均稳定。测试EXE SHA256为`D6A074A68402BC5BA7CFAF4C8DA45D86353BA127A053526DE90825F9F300DB8B`，228,219,392字节。它是debug测试程序，不是可安装的应用EXE。11个修改源码文件的精确哈希见`source-files.json`：`sha256`/`bytes`对应实际测试的Windows工作副本，`gitBlobSha1`对应Git规范化换行后的提交对象，其他系统checkout可能有换行差异；运行时源码没有因追加本文档而变化。

Windows x86_64、Rust1.99.1、原默认线程栈、每组`--test-threads=1`；145个不同的定向测试函数，**144通过、1失败**，不是全仓库全绿。每个实际测试名、运行回执编号与状态见`runtime-results.json`。

| 不重叠测试组 | 通过 | 失败 |
| --- | ---: | ---: |
| 新恢复准入与布局单元 | 5 | 0 |
| 既有入口future布局 | 1 | 0 |
| CLI观察器HTTP分块/压缩解码 | 4 | 0 |
| 新恢复HTTP集成 | 16 | 0 |
| TLS模块 | 14 | 0 |
| 明确选定的实际Codex CLI联测 | 2 | 0 |
| 完整bootstrap（含network恢复11项） | 42 | 0 |
| lifecycle | 26 | 0 |
| response_wire | 7 | 0 |
| request_upload HTTP | 9 | 1 |
| request_upload NACK | 4 | 0 |
| 大输入压缩 | 7 | 0 |
| events | 7 | 0 |

TLS14项中有一个测试函数内部覆盖8种TLS故障/HTTP协议/转换组合，不重复加8。两项单独运行的布局/上传用例已经包含在上表，不额外计数。现有frontend guard另6项通过，其源码未变化；它不是应用界面验收。

关键测试观测：

- 合成8MiB data URL与工具历史在两次上游请求中逐字节相同，最终完成；并非真实模型的图像理解测试。
- 三条小型合成请求同时到达，共享账号和client generation；只有失败流重发，发送数为2/1/1，全部完成，旧池退休一次、3个permit归还。没有把它说成生产5并发/71MB/200M压测，也没有证明同一物理H2连接故障不影响其余复用流。
- TLS坏record/缺close_notify、HTTP/1.1和HTTP/2、纯namespace转换均有实际环回注入。原始partial/business/冲突事件、hosted/未知工具拒绝恢复。
- 已发布metadata后的取消会停止恢复future并归还唯一permit；预算耗尽、第二次断流、恢复的HTTP/SSE拒绝均不启动第三次发送。
- 恢复完成后的下一独立请求保留最终response ID、原owner和完整历史。真实CLI两项结果见`cli-results.json`。

验收过程中发现并修正的缺陷也保留，不能仅呈现后来的绿色结果：

1. 首轮两项CLI观察器把chunked+zstd raw wire当UTF8而panic，当时没有完整CLI stdout回执，不能追认客户端成功。现已改成先保存raw/stdout/stderr/exit，再有界解chunked及gzip/zstd；4项decoder和2项真实CLI测试已通过，转发字节本身没有改变。
2. 首轮内联Candidate/reopen future使公共async状态增大，完整bootstrap里的`continuous_capacity_recovers_large_echoed_handshake_and_compact_requests`默认栈溢出。改为`Option<Box<Candidate>>`和`Box::pin`后，准确用例与完整bootstrap42项均通过，没有增大线程栈。不能据此解释历史生产TLS断线。
3. 24MiB上传用例`upload_http_real_gateway_uses_progress_before_first_response`在最终D6A完整模块中仍返回504/`upstream_connect_timeout`（9通过、1失败），同一artifact单独运行则通过。原1500ms opening、12MiB/s接收、100ms头延迟均未放宽，不能把单例通过当作已解决，也不能把初次失败唯一归因于Box。上传、execution及该测试源码相对base无改动。下面记录未修改base的对照，单独分析此问题。

### 上传回归基线对照

使用准确未修改的base `a243d9415da9a45a61dc9f98451c615bfad4ffcd`、clean detached checkout编译。上传测试文件两边SHA256均为`7E7CE9C7FDB03841F1278034BF5968F914780A2E6C68FD10065196644012DB87`。两边用相同隔离runner环境和原fixture参数，串行运行；最终修复版使用已保存且hash固定的D6A测试EXE，不重编或调参后偷换比较对象。

| 顺序/用途 | 官方base | 最终修复版D6A |
| --- | --- | --- |
| 最初定向模块运行 | 尚未运行 | `113849-run-996b68e5`：9通过1失败，20.96s |
| 单独准确上传用例 | `115322-baseline-run-ab922f2d`：1通过 | `113317-run-30b6fd3d`：1通过（早于上方模块） |
| 第一次整组交叉 | `115233-baseline-run-fd87e817`：10通过 | `115433-run-0e920d55`：10通过 |
| 预先限定额外配对1 | `115602-baseline-run-47c7351b`：10通过 | `115627-run-1edb16de`：10通过 |
| 预先限定额外配对2 | `115652-baseline-run-df0e14b6`：10通过 | `115717-run-187c7c3e`：10通过 |

后三轮每边整组耗时19.63–19.74s；基线artifact SHA256为`C9911146FC1D09EA2BCD233ED73815E61ADB95BAD5579FC099F5FF394C85B561`。有限样本排除了“修复版必定失败”，**未证明首次失败原因，也未证明官方base存在同一失败**。初版73ABA…另有一次9/1，只作为开发历史，不混成D6A两次失败。

这是需要维护者了解的验证限制，不写成“145项首次全绿”或“无性能回归”。24MiB/12MiB/s加100ms响应延迟理论约2.1s，完整网关准备和上传共用有界opening余量，debug机器调度/准备开销可能影响边界，但本次没有足够phase计时证据确认原因。源码检查显示新capture共享Bytes而不复制整份24MiB正文，最终入口future相较base仅+40B；这些证据同样不能代替故障瞬间计时。

若维护者再次复现该时限失败，建议保持原断言/参数，在测试响应结束后、断言前记录调用数、收完整body数量、local preparation和attempt计时，以区分准备、上传和等头阶段。此次未加生产日志或扩大超时掩盖问题。公开机器结果包含全部上述运行，而不是只取最后成功的一次。

开发库测试与frontend guard不等于可安装发布包。此次不提交EXE、不替换生产、不声称界面启动已验收；后续如制作安装包仍须通过Tauri前端guard，并验收同一个最终EXE的真实WebView渲染，不能使用缺失前端的旧产物。

## 参考与未解决事项

- Rustls关于TLS提前EOF的说明：https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof
- 当前客户端公开配置参考：https://learn.chatgpt.com/docs/config-file/config-reference
- 相关公开报告：https://github.com/jlcodes99/cockpit-tools/issues/3291 与 https://github.com/jlcodes99/cockpit-tools/issues/3349
- OpenAI公开background模式续流依赖真实response ID与cursor支持，不能假设Codex原生HTTP提供相同协议：https://developers.openai.com/api/docs/guides/background

尚未确认究竟由代理、无线链路、上游或其他网络组件引发各次TLS断开，也未取得历史请求最后业务帧。最终交付仅主张实际回执验证的有限恢复能力和明确安全边界，不主张消除一切断网或覆盖任意部分执行。真实用户场景是否符合准入条件，仍需后续正常使用的脱敏运行证据。
