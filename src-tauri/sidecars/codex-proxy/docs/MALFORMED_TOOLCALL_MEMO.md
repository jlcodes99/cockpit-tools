# 畸形 tool_call 问题备忘录

> 创建时间: 2025-12-19
> 状态: 待观察，暂不修复

## 问题描述

上游 Claude API 在流式返回时，偶尔会在同一个 `content_block` 中错误地发送多个工具调用的参数。

### 表现形式

1. **参数拼接**：两个工具的 JSON 参数被拼接成无效格式
   ```json
   {"command": "git diff --stat", "description": "..."}{"command": "git diff xxx", "description": "..."}
   ```

2. **元数据缺失**：第二个工具缺少必要的 `name` 和 `id`

3. **下游解析失败**：客户端（如 Claude Code）收到畸形数据后可能无法正确解析

### 日志示例

```
2025/12/19 11:00:51.203101 🛰️  上游流式响应合成内容:
...
Tool Call: Bash({"command": "git diff --stat", "description": "获取变更统计摘要"}{"command": "git diff backend-go/internal/providers/claude.go", "description": "获取 claude.go 的详细变更内容"}) [ID: toolu_01S6L3ngcGA9XKQrT1o2PLQa]
```

## 曾尝试的修复方案

### 方案 1: 实时流处理修复（已放弃）

在 SSE 流传输过程中实时检测并修复畸形数据。

**实现内容**：
- `toolCallFixer` 结构体跟踪状态
- `findJSONObjectBoundary()` 使用状态机检测 JSON 边界
- `inferToolName()` 根据参数推断工具名称
- `shouldFilterStop()` 过滤重复的 stop 事件
- `cleanupOnStop()` 清理内存

**放弃原因**：
- 逻辑复杂，经过多轮 Codex Review 仍有边缘问题
- 合成 block 的 index 可能与后续上游 index 冲突
- 需要处理重复 `content_block_stop` 事件
- 内存管理复杂

### 方案 2: 流结束后修复（未实现）

在流式响应完全结束后，检测并修复拼接的 tool_call。

**优势**：
- 逻辑简单：只在流结束时处理一次
- 无状态冲突：不需要实时跟踪 block index

**未实现原因**：
- 问题发生频率较低
- 等待上游修复

## 工具名称推断规则

如果将来需要实现修复，可根据参数 key 组合推断工具类型：

| 参数组合 | 工具名称 |
|---------|---------|
| `file_path` + `content` | Write |
| `file_path` + `old_string` + `new_string` | Edit |
| `file_path` (仅) | Read |
| `command` | Bash |
| `pattern` + `output_mode`/`glob`/`type` | Grep |
| `pattern` (仅) | Glob |
| `url` | WebFetch |
| `query` | WebSearch |
| `todos` | TodoWrite |
| `prompt` + `subagent_type` | Task |

## 相关文件

- `internal/providers/claude.go` - Claude Provider 流式处理
- `internal/utils/stream_synthesizer.go` - 日志合成器

## 后续行动

- [ ] 持续观察问题发生频率
- [ ] 如频繁触发，考虑实现"流结束后修复"方案
- [ ] 关注上游 Claude API 是否修复此问题
