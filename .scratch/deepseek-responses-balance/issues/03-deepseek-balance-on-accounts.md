# 03 — 在 Codex 账号页查询并展示 DeepSeek 余额

**What to build:** DeepSeek API Key 用户可以在现有 Codex 账号卡片、表格和详情中查看总余额、赠金余额与充值余额，并沿用当前刷新和缓存体验。

**Blocked by:** 01 — 让新 DeepSeek 账号原生运行 Codex Responses。

**Status:** ready-for-agent

- [ ] 使用 Bearer API Key 请求官方 `/user/balance`。
- [ ] 金额按官方十进制字符串保存，不使用浮点数计算。
- [ ] 简中与繁中首选 CNY，其他语言首选 USD，缺失时回退第一条真实币种。
- [ ] `is_available=false` 显示“余额不可用”，不显示金额且不视为错误。
- [ ] 查询失败不阻止账号导入、编辑、切换或启动。
- [ ] 导入后立即查询，账号事件遵循 10 分钟缓存门槛，保留手动刷新。
- [ ] 缓存恢复不丢失多币种余额。
- [ ] 账号页不显示无意义的今日请求或今日 Token 零值。
- [ ] Model Provider Usage Summary seam 的测试通过。
