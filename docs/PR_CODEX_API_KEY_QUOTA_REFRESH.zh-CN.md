# PR：同步 Codex API Key 供应商配额与余额

## 建议标题

`feat(codex): 同步 API Key 供应商配额与余额到 API 服务`

## 背景

Cockpit Tools 通过局域网提供 Codex API 服务时，远程客户端只能读取主机生成的额度快照。此前刷新流程主要覆盖 OAuth 账号，API Key 供应商余额可能长期停留在旧缓存；账号卡片、供应商管理页、Codex 注入页和 `/v1/cockpit/quota` 对余额的取值与格式也不一致。

本 PR 统一 API Key 供应商用量的查询、持久化、账号池汇总、刷新入口和前端展示，并将 `cockpit_tools` 返回的 API Key 余额默认解释为人民币。

## 主要改动

### 1. 统一供应商用量查询

- API Key 账号复用模型供应商配置中的 Base URL、API Key 和集成类型。
- 根地址先查询用户配置的地址；遇到 404 或自动识别失败时，再尝试同一主机的 `/v1`。
- 支持 `cockpit_tools`、`sub2api`、`new_api`、Token Plan 和既有 DeepSeek 兼容查询。
- 自动检测到的供应商集成类型会尽力保存；保存失败只记录告警，不会让已经成功的余额刷新回滚为失败。

### 2. 持久化并汇总 API Key 余额

- 查询成功后将完整摘要保存到账号的 `quota.raw_data.provider_usage`。
- sidecar 配额池读取缓存摘要，将 API Key 余额汇总到 `apiKeyBalance` 和 `API_KEY.balance`。
- `/v1/cockpit/quota` 可以同时返回 OAuth 时间窗口和 API Key 余额，不再把金额混入百分比窗口。
- API 服务账号集合使用全局账号池与各 `apiKeys[].accountIds` 的稳定去重并集，Key 独立账号池不会被遗漏。

### 3. 统一刷新入口与写入边界

- API 服务刷新同时处理 OAuth 账号和 API Key 账号，单个账号失败不会阻塞其他账号。
- 单账号刷新成功后更新一次 sidecar；批量刷新在整个批次结束后只更新一次。
- macOS 单账号、全部账号和 API 服务账号池刷新均复用统一刷新逻辑，删除重复的供应商查询与持久化实现。
- Codex 注入页的刷新按钮使用同一账号集合和刷新入口。

### 4. 人民币单位、精度与旧数据兼容

- `cockpit_tools` 的 `apiKeyBalance` 默认单位改为 `CNY`。
- `5.6` 展示为 `¥5.6`，`1234.567` 展示为 `¥1,234.57`。
- 账号页、供应商管理页和 Codex 注入页使用一致的人民币展示。
- 兼容升级前已经持久化的旧摘要：当 `mode` 为 `cockpit_tools`、旧 `unit` 为 `%` 且余额位于 `details.apiKeyBalance` 时，仍按人民币恢复余额。
- 账号页旧 localStorage 缓存不会覆盖时间更新的主机端余额。

### 5. 结构整理

- 提供共享的 `effective_api_service_account_ids` interface，集中账号池并集规则。
- `integrationType` 使用共享类型来源，避免页面、组件和供应商模块各自维护字符串联合。
- API Key 查询与持久化从 sidecar 刷新时机中拆开，让单账号和批量调用者明确控制写入边界。

## 完整行为变化

### 供应商查询与摘要

- 新增 `cockpit_tools` 集成类型；该类型直接请求供应商的 `/cockpit/quota`，不会进入 DeepSeek、Token Plan、`new_api` 或 `sub2api` 的自动识别分支。
- Cockpit Tools 远程查询使用 Bearer Token，默认请求路径为 `/v1/cockpit/quota`；如果 Base URL 已包含路径，则保留该路径并追加 `/cockpit/quota`。
- 远程查询限制为 10 秒超时、最大 256 KiB 响应；HTTP 错误正文会压缩空白并截断到 300 字符，避免错误信息和内存占用无界增长。
- 接受根对象、`data` 包装和 `data.quota`/`quota` 包装三种响应结构。
- Cockpit Tools 摘要包含：scope、5h/周剩余、API Key 余额、账号总数、可用/异常/冷却账号数、stale 状态及按计划汇总信息。
- Cockpit Tools 摘要的顶层 `balance` 等于 `apiKeyBalance`，`unit` 固定为 `CNY`；余额不会写入 `remaining`，避免与百分比语义混淆。
- 供应商根地址查询失败时，仅对 404、类型不支持和自动检测失败继续尝试 `/v1`；网络、鉴权等真实错误立即返回。
- 自动识别出的 `new_api`/`sub2api` 类型按供应商 ID 优先、标准化 Base URL 兜底保存。类型保存失败为 best-effort 告警，不影响已成功的余额查询和持久化。

### 账号持久化与新旧数据优先级

- 新增前端到 Rust 的 `codex_sync_api_key_usage_summary` 命令，并注册到 Tauri invoke handler。
- 摘要只允许写入 API Key 账号；空账号 ID、非 API Key 账号和非对象摘要会返回明确错误。
- 完整摘要写入 `quota.raw_data.provider_usage`，同时清理 `quota_error` 并更新 `usage_updated_at`。
- 如果账号尚无 quota，会创建不包含伪造百分比窗口的空 quota 容器。
- 带时间戳的旧摘要不会覆盖更新的数据；内容和时间均未变化时不会重复保存或重写 sidecar。
- 账号页用量缓存和后台批量刷新查询成功后都会尽力同步主机端账号摘要；同步失败只记录告警，仍保留前端查询结果。
- 读取余额时按模式选择字段：`new_api` 优先 `totalAvailable`，其他金额模式读取 remaining/balance/quotaRemaining；百分比模式不作为金额。
- 旧 Cockpit Tools 快照虽然错误保存为 `unit: "%"`，但只要 mode 为 `cockpit_tools`，仍会从顶层 balance 或 `details.apiKeyBalance` 恢复人民币余额。
- 账号页比较更新时间，避免旧 localStorage 用量状态覆盖主机端更新的 quota 快照。

### API 服务账号池与刷新

- 有效账号集合定义为 `collection.accountIds` 与所有 `collection.apiKeys[].accountIds` 的稳定去重并集。
- 该并集统一用于：运行时缓存裁剪、后台刷新、计费模型规则、菜单额度、可刷新账号列表、空池判断、sidecar auth 文件同步、sidecar 配置生成、Gateway 启动、账号健康和状态快照。
- 统一批量刷新入口先去重并过滤不存在/不允许刷新的账号，再按认证类型拆为 OAuth 与 API Key 两组。
- OAuth 继续调用 Codex quota 刷新；API Key 根据账号关联的供应商配置请求用量并持久化。
- 单账号失败只记录错误，其他账号继续；只要至少一个账号成功，批次就更新托盘和 sidecar 并返回成功数/参与数。
- 单账号入口成功后写一次 sidecar；批量入口在全部账号处理完后只写一次，避免 N 个 API Key 触发 N+1 次全量序列化和写盘。
- Codex 注入刷新、macOS 全部刷新和 macOS API 服务池刷新都改为调用统一批量入口。
- macOS 单账号 API Key 刷新复用统一查询/持久化入口；删除菜单模块内重复的供应商匹配、查询、摘要写入和类型保存实现。

### sidecar 配额池与 `/v1/cockpit/quota`

- sidecar 账号状态新增 `authKind` 与可选 `balance`；API Key 账号无需伪造 OAuth 时间窗口即可进入快照。
- API Key 账号不再从 `accountCount` 中排除，并按 `API_KEY` 计划聚合，而不是使用自定义供应商计划名。
- API Key 状态存在时计入 included/available；缺失状态仍计入 missing/abnormal。
- API Key 余额汇总到响应顶层 `apiKeyBalance` 和 `plans[].balance`，同时用 `updatedAt` 参与 stale 判断。
- OAuth 账号仍按实际窗口分别汇总 `fiveHourRemainingPercent` 和 `weeklyRemainingPercent`；API Key 不再产生 OAuth 窗口。
- 删除语义模糊的顶层 `remainingPercent`。调用方应分别读取 5h、周窗口和 API Key 余额。
- 运行时 auth health 修正可用/异常/冷却统计时，API Key 状态也会被识别为可用状态。
- Provider Gateway 上游回退只在当前 Key 的账号 scope 为空时触发；已有本地账号但缺少 quota 时保留隔离的本地空状态，不泄露其他 scope。
- Rust 生成的 sidecar 状态可从新摘要和既有多种 raw_data 结构恢复余额，包括 `provider_usage`、`usage.total_available`、`profile.usage` 和旧 balance 字段。

### 前端与 Codex 注入展示

- 新增 Cockpit Tools LAN 供应商预设，默认示例地址为 `http://127.0.0.1:60303/v1`，标记为服务型供应商。
- 选择该预设时自动设置 `integrationType: "cockpit_tools"`，并提示用户把示例地址替换成主机真实局域网 IP 和端口。
- API Key 账号卡片新增 Cockpit Tools 专用布局：5h、周、API Key 余额、账号池、可用账号、异常/冷却账号和 stale 状态。
- 账号详情、供应商卡片和供应商详情面板增加相同的 Cockpit Tools 核心指标，并把额外 plan 明细保留在详情列表中。
- API 服务页、API 服务弹窗、账号页的额度池摘要和详情弹窗都显示 API Key 余额。
- 上述页面计算成员和健康统计时，均包含每个局域网 Key 独立配置的账号 ID。
- API Key 金额最多保留两位小数和千位分隔；Cockpit Tools 默认带 `¥`，旧 `%` 元数据也归一为人民币。
- Codex 注入 footer 增加人民币余额 badge；详情卡的各计划也可以显示 balance。无可用余额时保持现有占位逻辑。
- `integrationType` 的完整联合集中为共享类型，并在账号页、供应商管理器、创建/更新/查询接口中复用。

### 多语言文案

- 全部 18 种语言（阿拉伯语、捷克语、德语、英语、en-US、西班牙语、法语、印尼语、意大利语、日语、韩语、波兰语、巴西葡萄牙语、俄语、土耳其语、越南语、简体中文和繁体中文）同步增加 API 服务余额、Cockpit Tools LAN 名称和配置提示。
- Codex 注入说明更新为包含账号数、可用账号、5h、周和 API_KEY 余额。
- 供应商用量字段增加 5h/周、账号池、可用/异常/冷却、stale 和 API Key 余额标签。

## 完整文件级变更清单

### 文档

- `docs/CODEX_API_KEY_QUOTA_CHANGES.zh-CN.md`
  - 新增功能背景、查询与持久化规则、API 返回示例、主要代码区域、验证结果和本地测试步骤。
  - 补充 Cockpit Tools 默认 CNY，并把精度示例更新为 `¥5.6`/`¥1,234.57`。
- `docs/PR_CODEX_API_KEY_QUOTA_REFRESH.zh-CN.md`
  - 本文件；整理可直接复制到 PR 的标题、背景、完整变更、接口变化、测试、风险和 checklist。

### Go sidecar

- `sidecars/cockpit-cliproxy/main.go`
  - 扩展 quota pool 状态和响应结构，加入认证类型、账号余额、顶层 API Key 余额和计划余额。
  - API Key 账号加入账号数、健康和 stale 统计，并独立于 OAuth 时间窗口聚合。
  - 移除 `remainingPercent`，调整 Provider Gateway 回退条件和 auth health 修正逻辑。
- `sidecars/cockpit-cliproxy/main_test.go`
  - 更新窗口、scope 隔离、上游失败和健康统计断言。
  - 新增 OAuth + API Key 混合池的余额、计划、账号数和窗口汇总测试。

### Rust/Tauri 后端

- `src-tauri/src/commands/codex.rs`
  - 新增 OAuth/API Key 统一批量刷新入口。
  - 新增摘要校验、时间戳防旧写、账号保存、sidecar 同步命令和供应商匹配/回退逻辑。
  - 新增 Cockpit Tools 查询与摘要适配，并在供应商用量命令中路由 `cockpit_tools`。
  - 新增持久化、URL fallback 和本地 HTTP CNY 摘要测试。
- `src-tauri/src/lib.rs`
  - 注册 `codex_sync_api_key_usage_summary` Tauri 命令。
- `src-tauri/src/modules/codex_remote_quota.rs`
  - 新增远程 Cockpit Tools quota adapter，负责 URL、鉴权、超时、响应大小、错误压缩、响应 envelope 和字段解析。
  - 包含 URL 拼接和 envelope 解析单元测试。
- `src-tauri/src/modules/mod.rs`
  - 导出 `codex_remote_quota` 模块。
- `src-tauri/src/modules/codex_local_access.rs`
  - 将有效账号并集提升为共享 interface，并替换所有 runtime/sidecar/健康/快照调用点。
  - sidecar 状态写入 auth kind、API Key balance 和 updatedAt。
  - 新增多结构余额解析、旧 Cockpit Tools `%` 快照兼容、账号并集和 sidecar 状态测试。
- `src-tauri/src/modules/codex_app_injection.rs`
  - quota 响应和计划结构加入余额。
  - 注入 footer/详情增加 `¥` 余额显示，刷新目标改用账号并集，刷新动作改用统一批量入口。
  - 更新脚本生成测试以覆盖余额常量、标签和人民币格式。
- `src-tauri/src/modules/macos_native_menu.rs`
  - 删除重复的供应商 Base URL 匹配、用量查询、摘要写入和 integration type 保存代码。
  - 单账号、全部账号和 API 服务账号池刷新均委托给 commands 层统一 interface。

### 前端页面与组件

- `src/components/CodexLocalAccessModal.tsx`
  - 额度池成员和健康统计包含 Key 独立账号池。
  - 计划摘要新增余额展示。
- `src/components/codex/CodexModelProviderManager.tsx`
  - 使用共享 integration type，并支持 `cockpit_tools`。
  - 供应商卡片和详情面板增加 Cockpit Tools 5h/周、余额及账号池指标。
  - API Key 余额使用共享人民币格式化并兼容旧 `%` 单位。
- `src/pages/CodexAccountsPage.tsx`
  - Cockpit Tools 预设自动绑定集成类型并显示 LAN 地址提示。
  - 新增账号卡片和详情的 Cockpit Tools 专用用量布局与字段格式化。
  - 查询缓存变化时同步完整摘要到主机账号，处理本地缓存与主机快照的更新时间优先级。
  - API 服务账号并集、健康统计、额度池摘要和详情均包含 Key 独立账号。
  - 创建/更新供应商时传递共享 integration type。
- `src/pages/CodexApiServicePage.tsx`
  - 成员列表包含 Key 独立账号；额度池计划增加余额文本。

### 前端服务与工具

- `src/services/codexApiKeyUsageRefreshService.ts`
  - 每个 API Key 用量查询成功后，把摘要和同一更新时间 best-effort 同步到 Rust 账号快照。
- `src/services/codexModelProviderService.ts`
  - 共享并导出完整 integration type，序列化/创建/更新/查询接口支持 `cockpit_tools`。
- `src/services/modelProviderUsageService.ts`
  - integration type/mode 加入 `cockpit_tools`。
  - 新增共享余额解析、摘要同步 invoke 和 Cockpit Tools 人民币格式化。
  - 兼容旧 `%` Cockpit Tools 摘要，其他百分比模式仍不会被当作金额。
- `src/services/modelProviderUsageService.test.ts`
  - 新增 CNY 精度、旧 `%` 兼容、不同模式余额选择和 Cockpit Tools mode 保留测试。
- `src/utils/codexProviderPresets.ts`
  - 新增 Cockpit Tools LAN 供应商常量、示例 Base URL 和预设。
- `src/utils/codexProviderPresets.test.ts`
  - 验证 Cockpit Tools 预设地址和服务标记。
- `src/utils/codexQuotaPool.ts`
  - quota pool item 新增 balance；API Key 只聚合金额，不生成 OAuth 窗口。
  - 支持从新旧 provider/raw_data 结构读取金额，并新增余额格式化。
- `src/utils/codexQuotaPool.test.ts`
  - 新增 API Key 余额、旧 Cockpit Tools 快照和不生成 OAuth 窗口测试；保留混合新旧窗口合并覆盖。

### 国际化

- `src/locales/ar.json`、`src/locales/cs.json`、`src/locales/de.json`
  - 同步阿拉伯语、捷克语和德语的余额、Cockpit Tools LAN 提示、账号健康及用量字段文案。
- `src/locales/en.json`、`src/locales/en-US.json`、`src/locales/es.json`、`src/locales/fr.json`
  - 同步英语、en-US、西班牙语和法语文案，并更新 Codex 注入功能说明。
- `src/locales/id.json`、`src/locales/it.json`、`src/locales/ja.json`、`src/locales/ko.json`
  - 同步印尼语、意大利语、日语和韩语文案。
- `src/locales/pl.json`、`src/locales/pt-br.json`、`src/locales/ru.json`
  - 同步波兰语、巴西葡萄牙语和俄语文案。
- `src/locales/tr.json`、`src/locales/vi.json`、`src/locales/zh-CN.json`、`src/locales/zh-tw.json`
  - 同步土耳其语、越南语、简体中文和繁体中文文案；简体中文文案同时明确人民币余额语义。

## 接口示例

```json
{
  "version": 1,
  "scope": "api_key_account_pool",
  "apiKeyBalance": 5.6,
  "accountCount": 2,
  "availableAccountCount": 2,
  "plans": [
    {
      "plan": "API_KEY",
      "count": 1,
      "balance": 5.6
    }
  ],
  "stale": false
}
```

`apiKeyBalance` 是账号池内 API Key 账号余额之和。该字段不会把 DeepSeek 余额或其他自定义供应商误判为 Cockpit Tools API Key 余额。

接口契约的另一项变化是：响应不再输出模糊的 `remainingPercent`。OAuth 使用方应读取 `fiveHourRemainingPercent` 和 `weeklyRemainingPercent`，API Key 使用方应读取 `apiKeyBalance` 或 `plans[].balance`。

## 验证

- `npm run typecheck`：通过。
- `node scripts/check_locales.cjs`：通过；18 份 locale JSON 均可解析、键集合一致，且每份均为 5675 个 key。
- `node --test src/services/modelProviderUsageService.test.ts`：9/9 通过。
- `codexQuotaPool.test.ts` 经 Vite SSR 运行：7/7 通过。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过。
- Cockpit Tools CNY 摘要、账号池并集、旧快照恢复和 Codex 注入人民币显示的 Rust 聚焦测试：通过。
- sidecar Go 配额测试：通过。
- `git diff --check`：通过，仅有 Windows CRLF 提示。
- 全量 Rust 测试在本机运行结果为 882 项通过、2 项忽略；另有 9 项既存的 Windows/文件锁环境失败。

本机未安装 `rustfmt`，因此未执行 `cargo fmt --check`。

## 手工验证建议

1. 在主机配置一个 `cockpit_tools` API Key 供应商，并将对应账号加入 API 服务账号池或某个 Key 的独立账号池。
2. 点击账号页、macOS 菜单或 Codex 注入页的额度刷新入口。
3. 确认账号页与供应商管理页显示 `¥`、千位分隔和最多两位小数。
4. 请求 `/v1/cockpit/quota`，确认 `apiKeyBalance` 与 `API_KEY.balance` 一致。
5. 使用升级前缓存的旧 `cockpit_tools` 摘要启动应用，确认无需先刷新也能恢复并显示人民币余额。

## 兼容性与风险

- OAuth 配额窗口的百分比计算和展示保持不变。
- 没有受支持用量接口的自定义供应商仍会返回原有的不可用错误，需要单独适配。
- 旧 `cockpit_tools` 摘要中的 `%` 只在专用 API Key 余额解析路径中被视为历史错误单位，不影响其他百分比配额。
- 批量刷新减少了重复 sidecar 序列化与写盘，但保留单账号刷新后立即更新快照的行为。

## Checklist

- [x] API Key 供应商余额查询与持久化
- [x] API 服务账号池并集与批量刷新
- [x] sidecar 和 `/v1/cockpit/quota` 余额汇总
- [x] CNY 展示及旧快照兼容
- [x] macOS 与 Codex 注入刷新入口统一
- [x] TypeScript、Rust、Go 相关验证
