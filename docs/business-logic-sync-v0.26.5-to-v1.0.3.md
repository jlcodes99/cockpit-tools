# v0.26.5 到 v1.0.3 业务逻辑迁移清单

整理时间：2026-07-04

## 目的

`v1.0.1` 到 `v1.0.3` 期间引入了平台热更新/独立平台包体系，同时也夹带了一批业务逻辑、UI 入口和内部机制修复。`v1.0.4` 已切回非热更新宿主内置体系，后续需要把有价值的业务改动逐步迁回当前代码。

本文只整理“业务逻辑和内部机制”，不把以下内容作为待迁移目标：

- 平台 zip 包生命周期、下载、安装、卸载、修复、bootstrap。
- `platform-packages/*` 远端索引、history、manifest、runtime。
- `crates/cockpit-*-adapter` 独立 adapter crate。
- `src/platform-ui/*` remote UI。
- 宿主加载远端 UI、平台包操作区、平台包不可用页。

对比基准：

- 稳定基线：`v0.26.5`
- 热更新问题版本：`v1.0.3`
- 当前非热更新分支：当前 `main` / `HEAD`

## 状态说明

- `已同步`：当前非热更新代码里已经能看到对应字段、命令、UI 或后端逻辑。
- `部分同步`：核心能力在，但缺少 `v1.0.3` 中的某些入口、持久化、错误处理或测试覆盖。
- `未同步`：当前非热更新代码里未看到对应实现。
- `不建议原样同步`：依赖热更新架构，应按宿主内置体系重新设计后再迁。

## 当前总览

当前 `main` 并不是纯 `v0.26.5`。已经回迁了一部分业务逻辑，例如：

- Codex 待授权 OAuth 账号。
- Codex 账号备注字段、2FA 秘钥、密码、手机号和其他备注。
- Codex 备注与通用 2FA 管理的基础联动。
- Codex `image_generation` 显式 `false` 持久化。
- Codex API Key / provider 保存的 patch 语义。
- Codex provider gateway 清理时保留非 Cockpit 管理的 `model_catalog_json`。
- 当前账号自动刷新间隔和账号级覆盖配置。
- 错误诊断和匿名错误上报开关。
- 后台 OAuth token 保活开关、启动延迟、平台级扫描节流和分批刷新。
- 顶部推广状态缓存，远端配置临时失败时不清空当前窗口状态。
- Codex 会话列表在 `session_index` 时间过期时回退真实 rollout 活动时间排序。
- 应用更新重启前 Codex API 服务关闭失败时继续重启，并在界面提示。
- 多个平台启动路径扫描和切号落盘修复。

本轮迁移后，Codex API 服务内部机制、会话迁移、批量删除后台任务、账号总览一键唤醒等核心业务能力已经回到当前非热更新体系。剩余主要是跨平台切号/启动路径的真实客户端回归，尤其 Windows 与实际官方客户端运行态。

## 需要后续迁移的业务改动

### 1. Codex 账号总览一键唤醒

状态：`已同步`

`v1.0.3` 行为：

- Codex 账号总览里有“唤醒账号”快捷入口。
- 入口会打开 Codex 唤醒测试流。
- 默认按 5h 额度从高到低挑选可唤醒 OAuth 账号。
- 如果当前筛选结果没有可唤醒账号，会提示“当前列表没有可唤醒的 OAuth 账号”。

当前状态：

- 当前代码有 Codex 唤醒任务页和测试能力：`src/components/codex/CodexWakeupContent.tsx`、`src/services/codexWakeupService.ts`、`src-tauri/src/modules/codex_wakeup.rs`。
- 账号总览页已增加“唤醒账号”快捷入口，点击后可进入 Codex 唤醒测试流。
- 总览入口会按当前筛选范围选取带 `refresh_token` 的 OAuth 账号，并默认按 5h 额度从高到低排序。
- 当前筛选范围没有可唤醒 OAuth 账号时，会在当前页面提示，不打开空测试弹框。

后续怎么迁：

- 已在当前 `src/pages/CodexAccountsPage.tsx` 的账号总览区域增加快捷按钮。
- 已复用现有 `CodexWakeupContent` 的测试弹框逻辑，没有新增第二套唤醒实现。
- 外部打开唤醒弹框时默认按 5h 额度降序排序。
- 当前筛选范围没有带 `refresh_token` 的 OAuth 账号时，在当前页面提示，不跳空弹框。

### 2. Codex API 服务固定首个账号路由策略

状态：`已同步`

`v1.0.3` 行为：

- `CodexLocalAccessRoutingStrategy` 新增 `SingleAccount` / `single_account`。
- API 服务可固定使用账号池里的第一个账号。
- 固定首个账号时不轮询其他账号，也限制失败重试只发生在该账号内。
- UI 调度策略下拉包含“固定首个账号”。

当前状态：

- 当前代码已有 `CodexLocalAccessRoutingStrategy::SingleAccount` / `single_account`。
- API 服务调度策略下拉已增加“固定首个账号”。
- 普通 HTTP 代理和 WebSocket 代理都会按同一有效策略计算最大凭据尝试次数。
- 选择固定首个账号时，账号顺序保持账号池原始顺序，并且单次请求最多只尝试首个账号；失败只在该账号内按单账号重试参数重试，不切到第二个账号。
- API key 限定账号范围时继续按限定范围的默认自动策略处理，不把集合级固定首个账号强套到部分账号范围。

后续怎么迁：

- 已在 `src-tauri/src/models/codex_local_access.rs` 增加路由枚举值。
- 已在 `src-tauri/src/modules/codex_local_access.rs` 的排序、sidecar 配置、请求循环中实现固定首个账号。
- 已在 `src/pages/CodexApiServicePage.tsx` 和翻译里补调度策略选项。
- 已补测试：固定首个账号不轮询、失败时不切到第二个账号、sidecar 参数输出为 `single_account`。

### 3. Codex Token 刷新跨进程文件锁

状态：`已同步`

`v1.0.3` 行为：

- Codex 受管 Token 刷新除了进程内 mutex，还增加跨进程文件锁。
- 锁文件路径类似 `.locks/token-refresh-<account>.lock`。
- 如果另一个进程已经推进 token 代际，当前进程复用新结果，避免重复刷新。
- 目标是减少 `refresh_token_reused`。

当前状态：

- 当前代码有 `token_generation`、进程内 `tokio::sync::Mutex`、受管投影版本校验。
- 当前代码已增加跨进程文件锁，锁路径在 Codex 账号目录下的 `.locks/token-refresh-*.lock`。
- 锁使用创建目录实现，不引入额外依赖；等待超时、stale lock 清理和 owner 元数据已补齐。
- 拿到锁后会重新读取账号；如果发现 `token_generation` 已经比调用方观察到的代际更新，且当前 `access_token` 未过期，则直接复用已完成刷新结果。
- 本次迁移只补并发保护和代际复用，不改当前非热更新版本的刷新触发策略。

后续怎么迁：

- 已给 `src-tauri/src/modules/codex_account.rs` 增加跨进程 token refresh lock。
- 锁粒度按 account id。
- 已保留现有 access-token-only、missing refresh token、受管投影回写逻辑。
- 已补测试：观察到旧代际时，若账号已被其它进程推进到新代际且 token 可用，则复用当前账号，不再强制发起刷新。

### 4. Codex API 服务历史请求日志价格重算

状态：`已同步`

`v1.0.3` 行为：

- `request_logs` 增加 `model_pricing_version`。
- usage event / request log 保存价格版本快照。
- API 服务提供“重算历史估值”入口，按当前价格表重算历史日志和统计金额。

当前状态：

- 当前代码保留了请求日志、usage stats、daily/weekly/monthly 统计。
- 当前代码已增加 `model_pricing_version`，集合价格表变化时递增版本。
- request log 会保存当时的价格版本、估算价值和输入/缓存输入/输出价格快照。
- 价格设置弹框已增加“重算历史估值”按钮，可按当前价格表重算 SQLite 历史请求日志，并重建本地统计快照。

后续怎么迁：

- 已迁移 SQLite schema，给旧日志补默认价格版本。
- 已在保存 usage event 时写入 pricing version。
- 已增加重算函数、Tauri command 和前端 service。
- 已在 `src/pages/CodexApiServicePage.tsx` 增加按钮和结果提示；失败显示在价格设置弹框内。
- 已补测试：重算后 cost 和 pricing version 更新，原始错误和 API key label 不丢。

### 5. Codex 批量删除后台任务

状态：`已同步`

`v1.0.3` 行为：

- 批量删除是后台 job。
- 页面显示进度条、已完成/总数、失败数和前几条失败详情。
- 支持暂停、继续、重试失败项、清理任务。
- adapter 或宿主重启后，运行中的删除任务恢复为 paused，避免破坏性删除自动继续。

当前状态：

- 当前宿主已提供 `start_codex_batch_delete`、`get_codex_batch_delete`、`pause_codex_batch_delete`、`resume_codex_batch_delete`、`retry_failed_codex_batch_delete`、`clear_codex_batch_delete`。
- 删除 job 快照落盘到 `codex_batch_delete_jobs`，运行中任务重启后恢复为 `paused`。
- `CodexAccountsPage.tsx` 启动后台任务后显示任务条、进度、失败数、前 5 条失败详情，并支持暂停、继续、重试失败项和清理任务。
- 删除成功后会刷新账号列表和分组，并清理 Codex API 服务账号池引用；账号池清理失败只记日志，不把已删除账号误判为删除失败。

后续怎么迁：

- 已在 `src-tauri/src/commands/codex.rs` 迁移宿主内置后台 job，不依赖 adapter。
- 已在 `src/services/codexService.ts`、`src/types/codex.ts`、`src/pages/CodexAccountsPage.tsx` 接入前端状态和轮询。
- 已补 18 个 locale 的批量删除任务文案。

### 6. Codex 批量导入任务恢复

状态：`已同步`

`v1.0.3` 行为：

- 批量导入扫描 session 会落盘到 `codex_batch_import_sessions`。
- 弹框关闭只是隐藏任务，不取消任务。
- 页面可恢复未完成 preview，重启后也能按 session id 恢复。

当前状态：

- 当前代码已有异步批量导入、progress event、resume、preview、confirm。
- 当前已增加 `CODEX_BATCH_IMPORT_SESSIONS_DIR` 和 session snapshot 落盘。
- 扫描开始、文件读取完成、逐项扫描进度、取消、继续、完成都会保存 session snapshot。
- `get_codex_batch_import_preview` 会先查内存，找不到再从磁盘恢复；恢复到的 `scanning` session 会降级为 `cancelled`，避免重启后自动继续扫描。
- 前端关闭批量导入弹框只隐藏任务，不取消任务；页面保留任务条，可重新打开。
- 前端会把当前 `sessionId` 保存到 `localStorage`，页面重载后可按 session id 恢复 preview。

后续怎么迁：

- 已在 `src-tauri/src/modules/codex_account.rs` 迁移 session snapshot。
- 已在 `src/pages/CodexAccountsPage.tsx` 迁移隐藏任务和 session 恢复。
- 已补 18 个 locale 的批量导入任务条文案。

### 7. Codex 会话迁移 Bundle

状态：`已同步`

`v1.0.3` 行为：

- 数据迁移支持 `codex_sessions` bundle。
- 导出 rollout 内容、`session_index` 条目和 workspace root。
- 导入时映射到本机实例目录，拒绝绝对路径、`..` 和非 rollout 文件。
- 导入后补 `session_index` / global state，并触发官方 metadata rebuild。

当前状态：

- 当前 `src/services/dataTransferService.ts` 已增加 `codex_sessions` bundle。
- 当前宿主已提供 `codex_session_transfer_export` / `codex_session_transfer_import`，不依赖 Codex adapter。
- 导出包含每个实例的 rollout 相对路径、rollout 内容、`session_index` 条目和 workspace root。
- 导入按本机实例 id 映射，默认实例可按默认目标映射；缺少目标实例会记录到导入 summary。
- 导入拒绝绝对路径、`..`、非 `sessions` / `archived_sessions` 目录和非 `rollout-*.jsonl` 文件。
- 导入会校验 rollout 内容中的 session id 与 bundle 记录一致，不覆盖本机已有 rollout。
- 导入后写入 `session_index.jsonl` 并触发官方 metadata rebuild；rebuild 失败会写入 summary，但 rollout/session_index 已落盘。

后续怎么迁：

- 已在 `src-tauri/src/modules/codex_session_manager.rs` 增加宿主内置 transfer bundle。
- 已在 `src-tauri/src/commands/codex_instance.rs` 和 `src-tauri/src/lib.rs` 注册 Tauri command。
- 已在 `src/services/dataTransferService.ts` 接入导出/导入。
- 已补测试：transfer 路径安全校验、rollout 内容 session id 校验。

### 8. Codex 会话可见性修复 dry-run 预览

状态：`已同步`

`v1.0.3` 行为：

- 会话可见性修复支持 `dry_run`。
- 预览只扫描和统计，不创建备份、不写 SQLite、不写 rollout、不重建 metadata。
- UI 是“预览变更 -> 确认修复”两步。

当前状态：

- 当前代码有 quick/deep 修复、进度、实例选择、provider 选择。
- `CodexSessionVisibilityRepairRequestOptions` 已支持 `dryRun`，后端 command 会映射到 `dry_run`。
- 当前弹框已支持“预览变更”和“开始修复”两条路径；预览完成后主按钮显示“确认修复”，但用户也可以不预览直接开始修复。

后续怎么迁：

- 已给修复 options 增加 `dry_run`。
- dry-run 扫描复用真实修复的统计逻辑，但跳过备份、SQLite 写入、rollout 写入、session_index 写入、metadata rebuild 和备份清理。
- 前端弹框支持“预览变更”，显示预计影响。
- 按业务要求，预览不是提交拦截；用户可以先预览，也可以直接点击“开始修复”执行真实修复。
- 已补测试：dry-run 不写会话文件、不改 SQLite、不生成备份。

### 9. Codex API 服务上游 401 后按 token 代际刷新再重试

状态：`已同步`

`v1.0.3` 行为：

- API 服务 HTTP 和 WebSocket 请求遇到上游 401 等认证失败时，会按观测到的 token generation 触发刷新。
- 如果 token 已由其他路径推进，则复用新 token，避免继续用旧凭证重试。

当前状态：

- 当前代码有 token generation、ensure fresh、单账号状态重试、错误分类。
- HTTP 和 WebSocket 上游 401 都会把当次请求观察到的 `account.token_generation` 传给刷新函数。
- 刷新函数改为 `force_refresh_managed_account_after_observed`；如果其它进程或路径已推进 token 代际且 access token 未过期，则直接复用新 token，不再强制重复刷新。

后续怎么迁：

- 已在 `src-tauri/src/modules/codex_local_access.rs` 对齐 `v1.0.3` 的 HTTP/WebSocket 401 retry 路径。
- 复用第 3 项已补的 token 代际复用测试覆盖核心刷新函数。

### 10. Codex provider / API Key 保存 patch 语义

状态：`已同步`

当前已看到：

- `bound_oauth_use_local_gateway` 显式 `false` 保留。
- 旧数据迁移只在字段缺失时启用默认迁移。
- provider gateway cleanup 只清理 Cockpit 管理的 `cockpit-provider-model-catalog.json`。
- API Key/provider 保存时保留已有模型目录、vision 能力、wire API 等元数据。

后续注意：

- 迁移其他 Codex 改动时不要重新引入“默认值重建整个账号”的保存方式。
- UI 保存备注、切号、API 服务启动、provider 测试都要走 patch 语义。

### 11. Codex 待授权 OAuth 账号和备注信息

状态：`已同步`

当前已看到：

- `create_pending_codex_oauth_account` command。
- `authorization_status=pending`。
- `account_note`、`two_factor_secret`、`account_password`、`phone_number`。
- 多行 JSON 数组导入待授权账号测试。
- 待授权账号卡片样式和备注弹框入口。
- 备注 2FA 可选择/写入通用 2FA 管理。

后续注意：

- 授权时应复用“重新授权”样式和交互。
- 待授权账号不能参与配额刷新、API 服务账号池、注入和套餐统计。

### 12. 外部链接导入 Codex 待授权账号

状态：`已同步`

当前已看到：

- `external_import` 支持 `provider=codex` / `platform=codex`。
- 支持 `activate`、`auto_import`、启动参数和 single-instance 参数处理。
- Codex import candidate 可识别 pending OAuth 数据。

后续注意：

- 浏览器批量导入时，待授权账号只能保存备注和邮箱，不应假装已授权。
- 导入错误要显示具体行号和原始解析错误。

### 13. 当前账号刷新设置和账号级覆盖

状态：`已同步`

当前已看到：

- `src/utils/currentAccountRefresh.ts`
- `CURRENT_ACCOUNT_REFRESH_PLATFORMS`
- `ACCOUNT_REFRESH_OVERRIDES_KEY`
- 设置页账号级刷新间隔覆盖。
- `useAutoRefresh` 使用账号级覆盖值。
- 数据迁移包含 `current_account_refresh_minutes`。

后续注意：

- 这只是“当前账号状态刷新”频率，不等同于 OAuth token 自动刷新策略。
- 后续改 token 刷新机制时不要混淆这两个概念。

### 14. 设置里的诊断/遥测错误上报

状态：`已同步`

当前已看到：

- `src-tauri/src/modules/diagnostics.rs`
- `get_diagnostics_config`、`save_diagnostics_config`
- `diagnostics_frontend_stage`、`diagnostics_frontend_ready`、`diagnostics_capture_event`
- 前端 `src/utils/errorReporter.ts`
- 配置项 `diagnostics_error_reporting_enabled` / `diagnostics_error_reporting_debug`

后续注意：

- 上报前必须继续做敏感信息脱敏。
- 账号 token、API Key、手机号、2FA 秘钥、密码、完整路径等不能上报。

### 15. 后台 OAuth token 保活轻量化

状态：`已同步`

`v1.0.1~v1.0.3` 行为：

- 后台授权保活不再启动后立刻扫所有平台账号。
- 只处理快过期授权，并按延迟、分批、失败退避执行。
- 设置里的后台保活开关变更后会立即同步到运行时状态。

当前状态：

- `provider_token_keeper` 已读取 `token_keeper_enabled`，关闭后跳过刷新扫描。
- 启动后默认延迟 5 分钟再跑首轮保活，设置变更可立即唤醒。
- 每个平台有独立扫描节流：无刷新时降低扫描频率，有刷新时保持短周期跟进。
- 每个平台单轮最多尝试 3 个刷新任务，避免大账号量时一次打满。
- 保留原有各平台真实刷新和当前账号回写逻辑，不改成 adapter 路径。
- 设置页已增加“后台授权保活”开关，保存后后端立即通知保活循环应用新状态。

后续注意：

- 这仍然是 OAuth token 保活，不等同于“当前账号状态刷新”间隔。
- 后续如果改 token 刷新策略，需要同时检查 Codex API 服务 401 刷新、账号页手动刷新和后台保活三条路径。

### 16. 顶部推广状态缓存和失败保留

状态：`已同步`

`v1.0.2` 行为：

- 顶部推广状态读取失败时不立即把当前窗口状态清空。
- 成功读取后缓存远端状态，后续启动可先展示缓存状态。

当前状态：

- `useTopRightAdStore` 已增加 `agtools.top_right_ad_state.cache.v1` 本地缓存。
- 读取成功会规范化 `ad` / `ads` 并写入缓存。
- 远端读取失败时保留当前状态，不再回退为空状态。

后续注意：

- 设置页的“显示顶部推广”只控制可见性，不改变远端推广数据缓存。
- 缓存写入失败不阻断主流程。

### 17. 各平台切号和启动路径修复

状态：`已同步`

`v1.0.1~v1.0.3` 涉及：

- Zed、Antigravity IDE、CodeBuddy、CodeBuddy CN、Cursor 的切号/启动稳定性修复。
- Antigravity IDE 切号启动可配置：关闭后只写账号配置，不启动或重启 IDE。
- 多平台启动路径扫描范围增强，尤其 Windows 下不同安装目录和可执行文件名。

当前状态：

- 当前 `process.rs` 已有统一 Windows app launch signature 扫描，覆盖 Antigravity IDE、Cursor、Zed、CodeBuddy、CodeBuddy CN、Qoder、Trae、WorkBuddy、Windsurf、Kiro、Codex、VS Code 等常见路径、命令名、协议名和显示名。
- 当前 `system.rs` 已有通用 app path scan root 与 `antigravity_launch_on_switch` 配置。
- 当前 Antigravity IDE 切号已支持“只写入默认账号数据，不关闭/启动/重启应用”的配置路径。
- Zed 与 `v1.0.3` 对比确认：业务逻辑同样使用 `/client/users/me`，401 不是简单漏迁旧逻辑；已增强 Zed 接口失败错误，失败时会保留安全的 `body_preview`，不再只有 `body_len`。
- Cursor 官方 API 请求错误链已补回当前宿主代码；token 刷新、user meta、stripe profile 和 usage 请求失败时会保留 reqwest 底层 source chain，并标注 timeout/connect/request 等诊断标签。
- CodeBuddy、CodeBuddy CN、WorkBuddy 当前会在切号时同步账号到默认客户端本地状态，当前账号读取也优先读真实客户端落盘结果，再用 `provider_current_state` 兜底。
- Cursor 当前账号读取优先读真实本地 auth，索引存在但详情全部丢失时会明确报错并避免空索引覆盖。
- 已补 Windows-only 单测约束：平台启动签名必须覆盖 Antigravity IDE、Cursor、Zed、CodeBuddy、CodeBuddy CN、Qoder、Trae、WorkBuddy、Windsurf、Kiro、Codex、VS Code；Antigravity IDE 签名不能误用 legacy `Antigravity.exe`。
- 已补通用当前账号映射单测：覆盖 `github-copilot` / `ghcp`、`codebuddy-cn` 别名规范化，以及账号被删除后 stale current id 会自动清理。
- 已补统一当前账号入口单测：`get_provider_current_account_id` 的后端分发覆盖 Windsurf、Kiro、Cursor、Gemini、CodeBuddy、CodeBuddy CN、Qoder、Trae、WorkBuddy、GitHub Copilot、Zed 以及 `codebuddy-cn`、`github-copilot`、`ghcp` 别名；未知平台会明确报错。
- 已补 Antigravity IDE 切号流程决策单测：legacy target 固定走 legacy；IDE target 在 `antigravity_launch_on_switch=false` 时优先走本地写入不启动流程，不会被 dual no-restart 配置覆盖；只有允许启动且启用 dual no-restart 时才走 dual no-restart。
- 代码层迁移已完成；仍需按平台逐个做真实客户端回归，尤其 Zed `/client/users/me` 401、Antigravity IDE 当前账号不变、Cursor OAuth 错误链。

后续怎么迁：

- 不迁平台热更新 adapter。
- 只迁真实落盘、当前账号解析、OAuth callback、启动路径解析和切号后读回校验。
- `v1.0.3` 的通用 SQLite `account_store` 属于平台包化期间的内部存储收敛，不在本轮原样迁移；当前非热更新主线继续以宿主内置 JSON 索引/详情文件和各客户端真实落盘读回为准。
- 每个平台单独列测试用例：添加账号、重新授权、切号、重启客户端、读取当前账号。
- 如果 Zed 继续出现 401，优先基于新增 `body_preview` 与本地 Keychain/Credential Manager 中的 user id、access token 来源排查；不要直接改掉 `/client/users/me`，因为 `v1.0.3` 也是该接口。

### 18. 多平台账号页批量选择工具栏

状态：`已同步`

`v1.0.2` 行为：

- Antigravity、Cursor、Gemini、GitHub Copilot、Kiro、Qoder、Trae、Windsurf、Zed、CodeBuddy 系列账号页使用统一批量选择工具栏。

当前状态：

- 当前已有 `src/components/AccountSelectionToolbar.tsx`。
- Codex 页面和账号总览已使用。
- Antigravity、Cursor、Gemini、GitHub Copilot、Kiro、Qoder、Trae、Windsurf、Zed、CodeBuddy、CodeBuddy CN、WorkBuddy 的独立账号页已接入统一批量选择工具栏。
- 顶部工具区不再在选中账号后单独塞一个批量删除按钮，批量删除入口统一放到选择工具栏。

后续怎么迁：

- 已对独立账号页按当前宿主内置页面结构接入 `AccountSelectionToolbar`。
- 保持批量删除、全选当前分页结果、清除选择的交互一致。

### 19. Codex 第三方模型 catalog 解析增强

状态：`已同步`

当前已看到：

- 模型列表解析支持常见 `data[]`、`models[]`、`slug`、`name` 等结构。
- provider gateway 写入模型 catalog。
- 清理时保留用户自定义 catalog。

后续注意：

- 新 provider 返回格式接入时，应统一走同一个 parser，不要在 UI 和后端各写一套。

### 20. API 服务错误分类和原始错误保留

状态：`已同步`

当前已看到：

- `codex_quota` 会追加 HTTP headers/body 诊断。
- `codex_local_access` 有较多错误分类、sidecar startup diagnostics、request log error fields。
- Cursor 等平台有原始错误链增强的部分改动。
- sidecar usage 事件会把 `request_failed` 且 message 命中 `upstream_response_failed` 的错误归一为 `upstream_response_failed`。
- 请求日志保留 `http_status`、`error_category`、`error_message`，WebSocket 错误也会提取 status/body。
- 已有测试覆盖 sidecar `request_failed` 覆盖为 `upstream_response_failed`、HTTP status 分类、WebSocket status 提取。

后续注意：

- 弹框内错误仍需按具体弹框逐个检查，新增弹框必须继续遵守“弹框内失败在弹框内提示”的项目规则。

### 21. Codex 会话排序回退真实活动时间

状态：`已同步`

`v1.0.3` 行为：

- Codex 会话列表不完全信任过期的 `session_index.jsonl` 时间。
- 当 `session_index.updated_at` 与 rollout 文件中的真实活动时间差距过大时，使用 rollout 活动时间排序。
- 目标是避免会话实际有新消息，但列表仍按旧索引时间排到后面。

当前状态：

- `codex_session_manager` 已增加 `SESSION_INDEX_ACTIVITY_DRIFT_SECONDS`，阈值为 1 小时。
- 会话快照时间解析改为 `resolve_thread_snapshot_updated_at_seconds`：
  - 优先读取 `session_index` 时间；
  - 如果与 rollout 活动时间相差超过 1 小时，回退使用 rollout 活动时间；
  - 如果两者都没有，再回退文件修改时间。
- 已补测试：过期 index 会使用 rollout 活动时间；接近真实活动时间的 index 会继续保留。

后续注意：

- 会话可见性修复里的 SQLite/session_index 校正仍保留现有逻辑；这里修的是会话列表读取和排序，不强制写回用户文件。

### 22. 应用更新重启不被 Codex API 服务关闭失败拦截

状态：`已同步`

`v1.0.3` 行为：

- 应用安装更新并准备重启前，会先尝试关闭 Codex API 服务和相关 sidecar。
- 如果关闭失败，不再阻断更新重启；界面提示“API 服务未正常关闭，将继续重启...”，日志记录 warning。

当前状态：

- `prepareCodexLocalAccessBeforeRelaunch` 已恢复为非阻断流程：
  - 成功关闭时记录 info；
  - 关闭失败时记录 warning；
  - 失败时展示继续重启提示；
  - 不再抛出错误阻断后续 install / relaunch。
- 已补 18 个 locale 的 `update_notification.stopApiServiceFailedContinue`。

后续注意：

- 如果 `install()` 或 `relaunch()` 自身失败，仍按原有更新错误流程提示；这里只是不把 API 服务关闭失败当成重启前置阻断。

### 23. 平台账号状态请求冷却和失败退避

状态：`已同步`

`v1.0.1` 行为：

- 后台账号状态维护不再反复请求已经失败的账号。
- 额度用完、临时限流和普通失败会被区分处理。
- 后台 token 保活按平台分批、节流和失败退避执行，减少大账号量场景的请求压力。

当前状态：

- `src-tauri/src/modules/quota.rs` 与 `v1.0.3` 的 `crates/cockpit-core/src/modules/quota.rs` 对比无差异，通用 quota 队列、退避和失败处理逻辑已经同步。
- `src-tauri/src/modules/quota_cache.rs` 与 `v1.0.3` 的 `crates/cockpit-core/src/modules/quota_cache.rs` 对比无差异，额度缓存行为已经同步。
- `provider_token_keeper` 保留了 `v1.0.3` 的业务策略：启动延迟、设置变更立即唤醒、平台级扫描节流、单平台单轮最多 3 个刷新任务、失败后 15 分钟退避。
- 当前实现按非热更新主线直接调用宿主内置账号模块，不走 `platform_adapter` 或平台包运行态检查。
- Codex API 服务仍保留账号健康、连续失败、冷却、429/额度限制分类和可见诊断，避免失败账号被密集重试。

后续注意：

- 这块是“请求压力控制”和“失败账号退避”，不是用户手动点刷新时必须静默吞错；手动刷新仍应保留可诊断的原始错误。
- 后续如果重新设计刷新策略，要同时检查后台保活、账号页批量刷新、API 服务请求重试和托盘刷新四条路径。

## 不建议原样同步的内容

以下改动和热更新体系强绑定，不建议直接迁回：

- 平台包安装/卸载/修复/更新 UI。
- 平台包 bootstrap 优先级和远端 UI 刷新事件。
- `platform_adapter` 通用 RPC 作为平台业务主路径。
- `crates/cockpit-core` 作为 adapter 共享 crate 的目录结构迁移。
- `src/platform-ui/*` remote module 的拆分方式。
- 平台包远端 index/history 发布规则。

如果后续重新做平台独立包，应单独立项，并先定义宿主/平台包边界、版本兼容、回滚策略和真实安装验证。

## 后续回归优先级

当前可在本地代码层面确认的 Codex 业务迁移已经落地，后续优先做真实客户端回归：

1. Zed 切号和 `/client/users/me` 401：重点核对 Keychain / Credential Manager 中的 user id、access token 来源，以及新增 `body_preview` 日志。
2. Antigravity IDE 当前账号独立逻辑：重点核对 `antigravity_launch_on_switch=false`、无重启切号、默认实例绑定账号和真实 IDE 当前账号读回。
3. Cursor OAuth / 切号错误链：重点核对 OAuth 完成、token 刷新失败、user meta / usage 请求失败时是否展示完整底层错误链。
4. CodeBuddy、CodeBuddy CN、Qoder、Trae、WorkBuddy、Windsurf：逐个平台验证添加账号、重新授权、切号、重启客户端、读取当前账号。

## 迁移时的原则

- 不迁热更新框架，只迁业务逻辑。
- 业务逻辑要落在当前宿主内置模块中，例如 `src-tauri/src/modules/*`、`src-tauri/src/commands/*`、`src/pages/*`。
- 切号必须以真实客户端落盘读回为准，不能只改前端 current id。
- Codex 账号保存继续使用 patch 语义，不能用默认值重建账号。
- 新增 UI 文案必须补齐 `zh-CN`、`en-US`、`en`，发版前跑 `node scripts/check_locales.cjs`。
- 高风险逻辑至少补单元测试或真实客户端回归步骤。

## 参考来源

- `v0.26.5` tag
- `v1.0.3` tag
- `v1.0.3` 的 `CHANGELOG.zh-CN.md`
- `v1.0.3` 的 `docs/codex-requirements-checklist-2026-07-03.md`
- 当前 `main` / `HEAD`
