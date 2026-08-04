# Cockpit Tools Provider Integration

This context describes how external model providers are represented and exposed to Codex users.

## Language

**DeepSeek 原生接入**:
Codex 通过 DeepSeek 官方 Responses API 使用 DeepSeek 模型；这是产品支持的唯一 DeepSeek 接入方式。
_Avoid_: DeepSeek Chat 网关、DeepSeek Chat Completions 接入

**首选余额币种**:
DeepSeek 多币种余额中用于客户端展示的一条记录。简体中文和繁体中文首选 CNY，其他语言首选 USD；目标币种缺失时使用接口返回的第一种实际币种。
_Avoid_: 默认币种、账户币种

**余额不可用**:
DeepSeek 成功返回余额信息但声明当前账户没有可供 API 调用的余额。这是账户状态，不是余额查询失败。
_Avoid_: 查询失败、接口错误
