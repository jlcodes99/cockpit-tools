# 02 — 自动迁移既有官方 DeepSeek 账号

**What to build:** 用户升级 Cockpit Tools 后，已有官方 DeepSeek 账号、供应商记录、默认 Codex 配置和受管实例自动切换到原生 Responses，无需手动修改。

**Blocked by:** 01 — 让新 DeepSeek 账号原生运行 Codex Responses。

**Status:** ready-for-agent

- [ ] 根路径和 `/v1` 的官方 DeepSeek 账号被幂等迁移到 canonical Base URL 与 Responses。
- [ ] 供应商记录固定为 DeepSeek usage 类型并保留 Pro、Flash。
- [ ] 当前启用的默认配置和受管实例立即获得新投影。
- [ ] 旧 provider gateway 状态和模型覆盖被安全清理。
- [ ] 迁移失败不会留下部分持久化结果。
- [ ] 自定义中转服务和非 DeepSeek 账号不被修改。
- [ ] 重复加载不会产生额外变化。
