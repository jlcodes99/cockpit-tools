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

## 验证

- `npm run typecheck`：通过。
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
