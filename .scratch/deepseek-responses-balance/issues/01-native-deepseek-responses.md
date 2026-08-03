# 01 — 让新 DeepSeek 账号原生运行 Codex Responses

**What to build:** 用户选择 DeepSeek 官方供应商并导入 API Key 后，可以直接使用 Responses API 启动 Codex，看到 Pro 与 Flash 两个模型，并在切出 DeepSeek 时安全恢复原模型。

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] 官方 DeepSeek Base URL 被规范化为 `https://api.deepseek.com`。
- [ ] 新账号固定使用 Responses direct，不启动 Chat Completions gateway。
- [ ] 模型目录包含 Pro 与 Flash 的官方 Codex 元数据。
- [ ] 无有效 DeepSeek 模型时默认 Pro，已有 Pro/Flash 选择保持不变。
- [ ] 首次切入时备份原模型，切出时只在用户未手动改模的情况下恢复。
- [ ] 同名但非官方主机的自定义中转服务保持原行为。
- [ ] 受管账号投影 seam 的测试通过。
