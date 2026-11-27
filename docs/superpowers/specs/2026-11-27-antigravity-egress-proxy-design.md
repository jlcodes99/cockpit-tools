# Antigravity 按账号出口代理（Egress Proxy）设计

日期：2026-11-27
状态：已批准并实现（UI 按 Codex 面板一致化；底层采用单值 egress_proxy_url 绑定）
分支：feat/provider-gateway-workbuddy

## 意图与背景

Codex 面板支持按账号绑定自定义出口代理（egress_proxy_url），Antigravity 面板没有这层能力，
只有「设置 → 网络 → 全局代理」。用户需要像 Codex 一样在 Antigravity 账号上选择/绑定固定代理，
并且全局代理目前覆盖不到 Antigravity 的 CLI 运行时（CLI 启动只是拼终端命令，不走 env 注入）。

## 目标 / 非目标

目标：
1. Antigravity 账号可保存 `egress_proxy_url`（可为空 = 跟随全局代理）
2. 桌面 IDE 实例与 CLI 运行时启动都注入账号代理（账号代理 > 全局代理）
3. CLI 启动补上全局代理注入（修复现有盲区）
4. 账号列表 UI 提供设置/清除代理入口

非目标（本期不做）：
- Antigravity OAuth 登录走代理（Google OAuth 令牌交换）
- 代理连通测试/出口国家检查（Codex 有，后续可复用其模式）
- 代理质量监测、按实例（非账号）代理

反目标：不改变 Codex 现有代理行为的任何一行。

## 方案取舍

- 方案 A（选定）：账号模型加 `egress_proxy_url` 字段，启动链路按「账号代理 > 全局代理」合并注入，
  UI 在 AccountsPage 的 Antigravity 账号卡片上加入口。改动集中、语义与 Codex 一致。
- 方案 B（否）：像 Codex 那样做独立的 proxy_targets 运行时绑定（desktop_target/runtime_id）。
  那套是为 Codex 本地网关/API 服务设计的，Antigravity 没有这层运行时，照搬会把复杂度引入一个
  没有对应架构的平台。
- 方案 C（否）：只做全局代理，不做按账号。不满足用户「像 Codex 一样按账号选」的核心诉求。

## 设计要点（已核对的锚点）

后端（src-tauri）：
1. 账号模型：Antigravity 账号记录加 `egress_proxy_url: Option<String>`
   （serde 默认 None，旧数据兼容）。持久化跟随现有账号存储写回路径。
2. 命令：`update_antigravity_account_egress_proxy(account_id, egress_proxy_url)`，
   校验协议 http/https/socks5/socks5h（对齐 Codex 的校验文案），空串 = 清除。
   在 lib.rs invoke_handler 注册。
3. 注入（复用 process_launch_candidates.rs 现有的 account_proxy_env_pairs 模式）：
   - 桌面 IDE：`start_antigravity_with_args` / `start_antigravity_legacy_with_args` /
     `start_managed_antigravity_on_macos` 增加账号代理参数；macOS `open --env` 链路与
     Windows/Linux Command env 链路都走「账号代理覆盖全局」的合并结果；
     NO_PROXY 沿用全局白名单 + 本地网关合并（merge_local_no_proxy）。
   - CLI：`account_cli_launch.rs::build_command` 生成命令时前缀 env 赋值
     （账号代理 > 全局代理 > 不加），顺带修复全局代理对 CLI 不生效的问题。
4. 切号链路：switch 时把当前账号代理带给启动函数（沿 antigravity_shared_switch /
   account.rs 现有参数传递路径）。

前端（src）：
1. `services/accountService.ts`：新增 `updateAntigravityAccountEgressProxy`。
2. **UI 与 Codex 面板保持一致**（用户明确要求）：交互与视觉对齐 Codex 账号页的代理体验——
   账号入口打开代理弹层/预览（对齐 CodexAccountProxyPreview），管理页对齐 CodexEgressProxyPage；
   优先直接复用/泛化现有 Codex 代理组件，而非另起一套样式。
3. locale：优先复用 codex.proxy.* 既有多语言 key（18 种语言现成翻译，保证 UI 文案完全一致），
   Antigravity 特有文案再补 antigravity 命名空间（zh-CN + en）。
4. 类型：账号类型加可选字段 `egress_proxy_url?: string | null`。

## 测试

- Rust：env 合并优先级单测（账号代理覆盖全局、空账号回落全局、NO_PROXY 合并），
  参照 process_tests.rs 的 build_managed_proxy_env_pairs 测试写法。
- 前端：tsc 通过；不新增组件测试（沿用现有页面手动验证路径）。
- 验证命令：`cargo test -p`（相关包）+ `npm run build`（或 tsc --noEmit）。

## 留给实现的决定

- 账号代理字段在账号 JSON 中的具体命名与写回函数，按现有存储代码习惯落位。
- UI 弹层复用项目现有 Modal/输入样式，不引入新依赖。
