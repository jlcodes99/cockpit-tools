# Codex API Key 配额与余额改动说明

## 背景

通过局域网使用 Cockpit Tools API 服务时，远程电脑只能看到主机保存的额度快照，无法直接看到主机上 API Key 供应商已经查询到的余额。此前 API 服务刷新入口也只刷新 OAuth 账号，API Key 余额可能停留在旧缓存中；另外，Cockpit Tools 类型的 `apiKeyBalance` 在账号页被当作整数格式化，接口返回 `5.6` 时界面显示为 `6`。

## 本次改动

### 1. API Key 供应商配置与账号卡片用量统一

- API Key 账号继续使用现有的 Codex 模型供应商配置，包括供应商、Base URL、API Key 和接口类型。
- API Key 余额刷新复用账号卡片已经使用的供应商用量查询逻辑，不再为 `aihub` 单独写供应商名称判断。
- 根地址供应商会先查询配置的地址；当接口返回 404 或自动识别失败时，会继续尝试同一主机的 `/v1` 地址。
- 支持的用量模式包括 `cockpit_tools`、`sub2api`、`new_api`、Token Plan，以及已有的 DeepSeek 兼容查询。完全自定义且没有受支持用量接口的供应商仍需要单独增加适配。
- `cockpit_tools` 局域网供应商返回的 `apiKeyBalance` 默认按人民币（`CNY`）解释和展示。

### 2. API Key 用量持久化到账号和 API 服务快照

- 查询成功后，将完整摘要写入 API Key 账号 `quota.raw_data.provider_usage`。
- sidecar 配额池读取该摘要并汇总 `apiKeyBalance`，同时在 `API_KEY` 计划中返回余额。
- 不再计算没有明确意义的所有类型“总体剩余”金额；OAuth 账号继续显示自己的时间窗口百分比，API Key 账号显示供应商余额。
- 账号页旧的 localStorage 用量缓存不能覆盖时间更新的主机端余额快照。

### 3. API 服务刷新同时刷新 OAuth 和 API Key

点击 Codex API 服务额度刷新按钮时：

- OAuth 账号按原有逻辑刷新 Codex 5 小时/周限额。
- API Key 账号读取关联供应商配置，主动请求供应商用量接口并更新余额。
- 多账号池中单个账号刷新失败不会阻止其他账号继续刷新。
- 刷新完成后更新 sidecar 配额池文件、托盘显示和 `/v1/cockpit/quota` 返回内容。
- macOS 菜单栏的 API 服务刷新入口也使用同一套账号池刷新逻辑。

### 4. `/v1/cockpit/quota` 返回 API Key 余额

API 服务配额接口现在可以返回：

```json
{
  "version": 1,
  "scope": "api_key_account_pool",
  "apiKeyBalance": 5.6,
  "accountCount": 2,
  "includedAccountCount": 2,
  "missingAccountCount": 0,
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

实际字段会根据账号池中的 OAuth、API Key 和可用数据动态返回；`apiKeyBalance` 是 API Key 账号余额之和，不代表 DeepSeek 余额，也不会把 AIHub 等自定义供应商误判成 DeepSeek。

### 5. 修复余额显示精度

- Cockpit Tools 类型的 `apiKeyBalance` 最多保留两位小数。
- `5.6` 显示为 `¥5.6`，不会再显示为 `6`。
- 较长金额会保留千位分隔，例如 `1234.567` 显示为 `¥1,234.57`。
- 账号数、请求数和百分比仍按整数显示。

## 主要代码区域

- `src-tauri/src/commands/codex_account_commands.rs`：OAuth/API Key 账号池统一刷新与刷新后的 sidecar 更新边界。
- `src-tauri/src/commands/codex_model_provider_commands.rs`：供应商用量查询、API Key 用量持久化和 Cockpit Tools CNY 摘要。
- `src-tauri/src/modules/codex_local_access_sidecar_config.rs`：有效账号并集、sidecar 配额池快照及余额恢复。
- `src-tauri/src/modules/codex_local_access_gateway_runtime.rs`：API 服务运行时缓存、准备进度和账号池刷新目标。
- `src-tauri/src/modules/codex_app_injection.rs`：Codex 页面中的 API 服务额度显示与刷新按钮。
- `sidecars/cockpit-cliproxy/relay_server.go`：sidecar 配额池响应中的 API Key 余额及计划汇总。
- `src/services/modelProviderUsageService.ts`：前端供应商用量查询、余额格式化和账号摘要同步。
- `src/services/codexApiKeyUsageRefreshService.ts`：API Key 用量刷新缓存及刷新后同步。
- `src/pages/useCodexAccountsAccessController.tsx`、`src/pages/useCodexAccountsRenderers.tsx`：账号页查询、缓存同步和用量显示。
- `src/components/codex/CodexModelProviderManager.tsx`、`src/components/codex/CodexModelProviderManagerView.tsx`：供应商管理页的查询与用量展示。
- `src/utils/codexProviderPresets.ts`、`src/services/codexModelProviderService.ts`：供应商模板、配置和接口类型。
- `src/locales/*.json`：API Key 余额、账号池和 API 服务用量相关文案。

## 验证结果

- `node --test src/services/modelProviderUsageService.test.ts`：9 项通过。
- `npm run typecheck`：通过。
- Rust API Key 用量持久化、`/v1` 地址回退、sidecar 余额汇总和 Codex 注入显示测试：通过。
- `git diff --check`：通过；仅有 Windows 换行符提示。
- `npm run tauri -- build`：已生成 MSI 和 NSIS 安装包。

## 本地测试步骤

1. 在提供 API 服务的主机安装新版 Cockpit Tools。
2. 确认 API Key 账号已经加入 API 服务账号池，且账号页能够显示供应商余额。
3. 重启 Cockpit Tools 和 Codex。
4. 点击 Codex API 服务额度旁边的刷新按钮。
5. 检查账号页显示的小数余额，并再次请求 `/v1/cockpit/quota`。

本机打包时配置中存在 Tauri 公钥，但未配置对应的 `TAURI_SIGNING_PRIVATE_KEY`，因此构建最后会提示无法生成更新签名；MSI 和 NSIS 安装包本身已经正常生成，可用于本地安装测试。
