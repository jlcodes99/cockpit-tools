# Codex profile 复制回归测试

此独立 Rust 测试包直接编译宿主的 `codex_profile_copy.rs`，覆盖 SQLite WAL 快照、会话路径与本机项目映射、分页历史和索引边界、来源保护、目标重叠以及取消后重试。

在项目根目录运行：

```powershell
cargo test --manifest-path tests/codex-profile-copy/Cargo.toml --locked --lib
```

Windows 的路径写法由对应条件测试覆盖；Unix 的权限与链接测试仅在 Unix 上执行。宿主的创建流程、登记锁并发与取消行为另由 `codex_instance_profile_copy_tests.rs` 验证。测试只读写隔离的临时夹具，不启动客户端或操作图形界面。
