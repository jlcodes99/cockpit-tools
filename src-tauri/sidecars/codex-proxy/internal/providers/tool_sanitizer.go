package providers

import (
	"encoding/json"
	"strings"
)

// sanitizeClaudeToolInput 用于修复上游模型偶发输出的“可选字段空字符串”问题。
// 目前已知 Read 工具的 pages 字段为空字符串会导致 Claude Code 工具执行器报错：
// <tool_use_error>Invalid pages parameter: ""</tool_use_error>
func sanitizeClaudeToolInput(toolName string, input interface{}) interface{} {
	if toolName != "Read" {
		return input
	}

	inputMap, ok := input.(map[string]interface{})
	if !ok {
		return input
	}

	if pages, ok := inputMap["pages"].(string); ok && strings.TrimSpace(pages) == "" {
		delete(inputMap, "pages")
	}

	return inputMap
}

func sanitizeClaudeToolArgsJSON(toolName string, argsJSON string) string {
	if strings.TrimSpace(argsJSON) == "" {
		return argsJSON
	}

	var input interface{}
	if err := json.Unmarshal([]byte(argsJSON), &input); err != nil {
		return argsJSON
	}

	sanitized := sanitizeClaudeToolInput(toolName, input)
	out, err := json.Marshal(sanitized)
	if err != nil {
		return argsJSON
	}
	return string(out)
}
