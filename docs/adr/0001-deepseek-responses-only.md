---
status: accepted
---

# DeepSeek 仅使用 Responses 协议

DeepSeek 曾按 Chat Completions 供应商通过本地网关接入。现在统一改为官方 Responses API：新账号直接使用 Responses，既有 DeepSeek 账号在升级后自动迁移，不提供协议切换入口，并保留 `deepseek-v4-flash` 与 `deepseek-v4-pro` 两个模型。迁移时保留账号已经选择的 Pro 或 Flash；新账号或无有效 DeepSeek 模型的账号默认使用 Pro。首次切入 DeepSeek 时备份原模型；切出时仅在当前模型仍为 DeepSeek 模型的情况下恢复，避免覆盖用户之后的手动选择。虽然官方接入页面在决策时仍提示 Pro 支持稍后开放，产品选择直接支持两者，以保持 DeepSeek 配置单一且无需后续模型迁移。
