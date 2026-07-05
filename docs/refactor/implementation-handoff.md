# Cockpit Tools Refactor Implementation Handoff

记录日期：2026-07-06
实现仓库：`E:\cockpit 重构\cockpit-tools-refactor-impl`
实现分支：`codex/refactor-ccs-integration`
基线：Cockpit Tools `v1.0.4` (`20511e4e7ec820cfc93984c62dc0daf33bfd718c`)

## 输入整合

已整合 `docs/refactor/thread-a-architecture-test-harness.md` 到 `thread-e-storage-backup-sync.md` 的只读分析：

- Thread A：首批重构应先冻结 baseline，再拆 `lib.rs` 启动 helper、`commands/codex.rs` 命令门面和 `codex_local_access.rs` 纯逻辑。
- Thread B：provider/model catalog 应先建立结构化 schema，兼容旧 `string[]`，后续再接 native Responses clean template、`web_search` 哨兵和 official auth 保护。
- Thread C：gateway/failover 短期保持“Rust 决策与持久化，Go sidecar 执行热路径”，第一批不复制 CC Switch proxy。
- Thread D：前端首批只抽纯工具函数和 props-only 边界，不改变 Codex 页面交互。
- Thread E：Cockpit 已有 `atomic_write.rs`；第一批最小 storage 修补是把 `codex_model_providers.json` 和 `codex_account_groups.json` 从 plain write 改为 atomic/backup/recovery。

## 已完成

1. 建立 baseline 验证文档和脚本。
   - 新增 `docs/refactor/baseline-verification.md`。
   - 新增 `scripts/refactor/verify-baseline.ps1`。
   - 脚本会记录 git/node/npm/cargo/rustc/go 版本，执行 `npm run typecheck`、`cargo test --workspace`、sidecar `go test ./...`，并把日志写入 `docs/refactor/verification-logs/*.log`。

2. 做了一个低风险启动拆分。
   - 新增 `src-tauri/src/app_bootstrap/mod.rs`。
   - 新增 `src-tauri/src/app_bootstrap/platform.rs`。
   - 从 `src-tauri/src/lib.rs` 移出 `raise_process_file_descriptor_limit`、`apply_startup_minimized`、`apply_macos_activation_policy`。
   - 保持原调用点和调用顺序不变，没有改 Tauri command registry。

3. 修补 Codex provider/group JSON store 的写入护栏。
   - `save_codex_account_groups` 和 `save_codex_model_providers` 现在先校验 JSON，再走 `modules::atomic_write::write_string_atomic`。
   - `load_codex_account_groups` 和 `load_codex_model_providers` 现在会经 `parse_json_with_auto_restore` 校验并可从 `.bak` 恢复。
   - 增加了 Rust 单测覆盖无效 JSON 不覆盖旧文件、替换前写 `.bak`、当前文件损坏时从 `.bak` 恢复。

4. 建立 provider/model catalog/common config 的最小代码骨架。
   - 新增前端纯工具 `src/utils/codexProviderCatalog.ts`，兼容旧 `string[]` 和结构化 `models[]`。
   - 新增 Rust 纯类型模块 `src-tauri/src/modules/codex_provider_contract.rs`，包含 catalog profile、wire API、catalog entry 和 common config snippet。
   - Rust 模块尚未接入运行时，只作为后续 Thread B/C/E 的稳定落点。

5. 补齐 Codex 前端大页面的首个纯工具边界。
   - 新增 `src/utils/codexAccountsOverview.ts`。
   - 从 `src/pages/CodexAccountsPage.tsx` 抽出 tier count 和 OAuth 绑定标签收集逻辑。
   - 页面仍保留原交互、状态流和 JSX；本轮不做大规模组件搬迁。

## 验证结果

已执行：

| 命令 | 结果 |
|---|---|
| `npm install` | 通过 |
| `npm run typecheck` | 通过；前端纯工具拆分后已复跑 |
| `powershell -ExecutionPolicy Bypass -File .\scripts\refactor\verify-baseline.ps1 -SkipNpmInstall` | 按预期返回非零：TypeScript 通过，Cargo/Go 缺失 |

当前环境阻塞：

- `cargo` 未安装或不在 PATH。
- `rustc` 未安装或不在 PATH。
- `go` 未安装或不在 PATH。

因此 Rust 单测、Rust workspace test 和 Go sidecar test 尚未在本机执行。相关 Rust 改动需要在装好 Rust/Go 后复验。

## 未做事项

- 未大规模拆 `commands/codex.rs` 或 `codex_local_access.rs`。
- 未接入 native Responses catalog generator、`web_search = "disabled"` 哨兵、official auth config-only 保护。
- 未实现 provider failover/circuit breaker。
- 未大规模拆 Codex 前端大页面组件/hook；本轮只抽出 catalog 与 overview tier/tag 的纯工具落点。
- 未做 operation snapshot、transaction rollback、SQLite 迁移或 cloud sync。

## 下一步建议

1. 在实现仓库安装 Rust 和 Go 工具链后先跑：

   ```powershell
   cargo test --workspace
   cd sidecars/cockpit-cliproxy
   go test ./...
   ```

2. 若 Rust 编译通过，继续首批小 PR：
   - 拆 `commands/codex.rs` 的 `codex_local_access_*` wrappers 到 `commands/codex/local_access.rs`，用 `pub use` 保持 command registry 路径。
   - 把 provider catalog skeleton 接入 `codex_protocol.rs` 的 generator 单测，但仍不改 UI 行为。
   - 将 `codex_model_providers.json` / `codex_account_groups.json` 的手工回归加入 baseline checklist。

3. 若 Rust 编译失败，优先修本轮新增的：
   - `src-tauri/src/app_bootstrap/platform.rs`
   - `src-tauri/src/modules/codex_provider_contract.rs`
   - `src-tauri/src/commands/codex.rs` 中的 JSON store helper/tests

## Git 状态要求

完成提交/推送后，请以 `git status --short --branch` 确认本地工作区干净。
