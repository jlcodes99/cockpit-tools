package utils

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestTruncateJSONIntelligently(t *testing.T) {
	tests := []struct {
		name           string
		input          interface{}
		maxTextLength  int
		expectTruncate bool
	}{
		{
			name:           "短字符串不截断",
			input:          "Hello",
			maxTextLength:  10,
			expectTruncate: false,
		},
		{
			name:           "长字符串截断",
			input:          strings.Repeat("a", 600),
			maxTextLength:  500,
			expectTruncate: true,
		},
		{
			name: "嵌套对象中的长字符串",
			input: map[string]interface{}{
				"short": "test",
				"long":  strings.Repeat("b", 600),
			},
			maxTextLength:  500,
			expectTruncate: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := TruncateJSONIntelligently(tt.input, tt.maxTextLength)

			// 转换为JSON检查
			jsonBytes, err := json.Marshal(result)
			if err != nil {
				t.Fatalf("Marshal failed: %v", err)
			}

			resultStr := string(jsonBytes)
			if tt.expectTruncate {
				if !strings.Contains(resultStr, "...") {
					t.Errorf("Expected truncation marker '...' not found")
				}
			}
		})
	}
}

func TestSimplifyToolsArray(t *testing.T) {
	tests := []struct {
		name     string
		input    interface{}
		expected string
	}{
		{
			name: "Claude格式工具",
			input: map[string]interface{}{
				"tools": []interface{}{
					map[string]interface{}{
						"name":        "get_weather",
						"description": "Get weather info",
					},
					map[string]interface{}{
						"name":        "search",
						"description": "Search the web",
					},
				},
			},
			expected: `["get_weather","search"]`,
		},
		{
			name: "OpenAI格式工具",
			input: map[string]interface{}{
				"tools": []interface{}{
					map[string]interface{}{
						"type": "function",
						"function": map[string]interface{}{
							"name":        "calculator",
							"description": "Calculate math",
						},
					},
				},
			},
			expected: `["calculator"]`,
		},
		{
			name: "非工具字段不受影响",
			input: map[string]interface{}{
				"model":    "claude-3",
				"messages": []interface{}{"hello"},
			},
			expected: `{"messages":["hello"],"model":"claude-3"}`,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := SimplifyToolsArray(tt.input)

			// 提取tools字段检查
			if resultMap, ok := result.(map[string]interface{}); ok {
				if tools, exists := resultMap["tools"]; exists {
					toolsJSON, _ := json.Marshal(tools)
					if !strings.Contains(string(toolsJSON), tt.expected) {
						t.Errorf("Expected tools to contain %s, got %s", tt.expected, string(toolsJSON))
					}
				}
			}
		})
	}
}

func TestFormatJSONForLog(t *testing.T) {
	input := map[string]interface{}{
		"model": "claude-3",
		"tools": []interface{}{
			map[string]interface{}{
				"name":        "get_weather",
				"description": "Get weather information",
				"parameters": map[string]interface{}{
					"type": "object",
					"properties": map[string]interface{}{
						"location": map[string]interface{}{
							"type":        "string",
							"description": "City name",
						},
					},
				},
			},
		},
		"messages": []interface{}{
			map[string]interface{}{
				"role":    "user",
				"content": strings.Repeat("Hello ", 200), // 长内容
			},
		},
	}

	result := FormatJSONForLog(input, 100)

	// 检查tools被简化
	if !strings.Contains(result, `"get_weather"`) {
		t.Error("Tools should be simplified to names")
	}

	// 检查长文本被截断
	if !strings.Contains(result, "...") {
		t.Error("Long content should be truncated")
	}

	// 检查JSON格式化
	if !strings.Contains(result, "\n") {
		t.Error("Output should be formatted with newlines")
	}
}

func TestMaskAPIKey(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		expected string
	}{
		{
			name:     "空字符串",
			input:    "",
			expected: "",
		},
		{
			name:     "短密钥(5字符)",
			input:    "abc12",
			expected: "***", // 长度<=5时返回***
		},
		{
			name:     "短密钥(6字符)",
			input:    "abc123",
			expected: "abc***23",
		},
		{
			name:     "长密钥",
			input:    "sk-1234567890abcdef",
			expected: "sk-12345***bcdef",
		},
		{
			name:     "超短密钥",
			input:    "key",
			expected: "***",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := MaskAPIKey(tt.input)
			if result != tt.expected {
				t.Errorf("Expected %s, got %s", tt.expected, result)
			}
		})
	}
}
