# Cockpit Tools Refactor Baseline Verification

记录日期：2026-07-06
实现仓库：`E:\cockpit 重构\cockpit-tools-refactor-impl`
基线来源：`jlcodes99/cockpit-tools` tag `v1.0.4`，commit `20511e4e7ec820cfc93984c62dc0daf33bfd718c`

## 当前结论

- `npm install` 已完成。
- `npm run typecheck` 在基线和首批改动后均通过。
- 当前机器 PATH 中没有 `cargo`、`rustc`、`go`，因此 Rust 和 Go 测试未能执行。
- 工作区路径包含中文和空格：`E:\cockpit 重构`。后续排查 Rust/Go/sidecar 构建时需要把 Windows 非 ASCII 路径作为验证项。

## 一键验证脚本

```powershell
cd "E:\cockpit 重构\cockpit-tools-refactor-impl"
powershell -ExecutionPolicy Bypass -File .\scripts\refactor\verify-baseline.ps1
```

如依赖已经安装，可跳过 `npm install`：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\refactor\verify-baseline.ps1 -SkipNpmInstall
```

脚本会把日志写到：

```text
docs/refactor/verification-logs/<timestamp>-baseline.log
```

## 已执行命令

| 命令 | 结果 | 备注 |
|---|---|---|
| `npm install` | 通过 | added 179 packages |
| `npm run typecheck` | 通过 | `tsc --noEmit` 无输出 |
| `cargo test --workspace` | 未执行 | `cargo` command not found |
| `go test ./...` | 未执行 | `go` command not found |

## 必跑回归矩阵

后续有 Rust/Go 工具链后，至少执行：

```powershell
cd "E:\cockpit 重构\cockpit-tools-refactor-impl"
npm run typecheck
cargo test --workspace
cd "E:\cockpit 重构\cockpit-tools-refactor-impl\sidecars\cockpit-cliproxy"
go test ./...
```

重点手工回归：

- Codex OAuth 账号切换。
- Codex API key 账号切换。
- OAuth 与 API key 互切后 `~/.codex/auth.json`、`~/.codex/config.toml`、Cockpit current account 一致。
- Codex API service/local gateway 启停。
- provider gateway manifest/config 生成。
- Go sidecar `/v1/responses`、`/v1/chat/completions`、SSE 转换和 usage 事件。
- `codex_model_providers.json`、`codex_account_groups.json` 保存失败时不覆盖旧文件，可从 `.bak` 恢复。

## 第一批改造护栏

- 不修改 `cockpit-tools-v1.0.4` 和 `cc-switch-v3.16.5` 只读快照。
- 不改变 Tauri command 名称、参数和返回 JSON shape。
- 不改变 local gateway、provider gateway、账号切换的运行时行为。
- 新增 provider/catalog/common config 类型只能作为纯逻辑落点，不接入写入链路。
- 所有配置写入必须保留未知字段；任何敏感文件只允许本机备份，不进入云同步。
