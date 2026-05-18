package utils

import (
	"encoding/json"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

func TestEstimateTokens(t *testing.T) {
	tests := []struct {
		name     string
		text     string
		expected int
	}{
		{"empty", "", 0},
		{"english", "Hello world", 3}, // ~11 chars / 3.5 = ~3
		{"chinese", "你好世界", 2},        // 4 chars / 1.5 = ~2.7 -> 3
		{"mixed", "Hello 你好", 3},      // 5 other + 2 cjk = ~1.4 + ~1.3 = ~3
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := EstimateTokens(tt.text)
			// 允许 ±2 的误差
			if result < tt.expected-2 || result > tt.expected+2 {
				t.Errorf("EstimateTokens(%q) = %d, want ~%d", tt.text, result, tt.expected)
			}
		})
	}
}

func TestEstimateResponsesRequestTokens(t *testing.T) {
	tests := []struct {
		name        string
		request     map[string]interface{}
		minExpected int
	}{
		{
			name: "simple_string_input",
			request: map[string]interface{}{
				"model": "gpt-4",
				"input": "Hello, how are you?",
			},
			minExpected: 5,
		},
		{
			name: "with_instructions",
			request: map[string]interface{}{
				"model":        "gpt-4",
				"instructions": "You are a helpful assistant.",
				"input":        "Hello",
			},
			minExpected: 8,
		},
		{
			name: "with_array_input",
			request: map[string]interface{}{
				"model": "gpt-4",
				"input": []interface{}{
					map[string]interface{}{
						"type":    "message",
						"role":    "user",
						"content": "Hello, how are you today?",
					},
				},
			},
			minExpected: 6,
		},
		{
			name: "with_tools",
			request: map[string]interface{}{
				"model": "gpt-4",
				"input": "Use the tool",
				"tools": []interface{}{
					map[string]interface{}{"name": "search"},
					map[string]interface{}{"name": "compute"},
				},
			},
			minExpected: 300, // 2 tools * 150 = 300
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			bodyBytes, _ := json.Marshal(tt.request)
			result := EstimateResponsesRequestTokens(bodyBytes)
			if result < tt.minExpected {
				t.Errorf("EstimateResponsesRequestTokens() = %d, want >= %d", result, tt.minExpected)
			}
		})
	}
}

func TestEstimateResponsesOutputTokens(t *testing.T) {
	tests := []struct {
		name        string
		output      interface{}
		minExpected int
	}{
		{
			name:        "nil_output",
			output:      nil,
			minExpected: 0,
		},
		{
			name: "message_with_text",
			output: []interface{}{
				map[string]interface{}{
					"type": "message",
					"content": []interface{}{
						map[string]interface{}{
							"type": "output_text",
							"text": "Hello, I am doing well!",
						},
					},
				},
			},
			minExpected: 5,
		},
		{
			name: "function_call",
			output: []interface{}{
				map[string]interface{}{
					"type":      "function_call",
					"name":      "search",
					"arguments": `{"query": "weather"}`,
				},
			},
			minExpected: 5,
		},
		{
			name: "reasoning_with_summary",
			output: []interface{}{
				map[string]interface{}{
					"type": "reasoning",
					"summary": []interface{}{
						map[string]interface{}{
							"type": "summary_text",
							"text": "This is my reasoning process",
						},
					},
				},
			},
			minExpected: 5,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := EstimateResponsesOutputTokens(tt.output)
			if result < tt.minExpected {
				t.Errorf("EstimateResponsesOutputTokens() = %d, want >= %d", result, tt.minExpected)
			}
		})
	}
}

func TestEstimateResponsesOutputTokensWithTypedItems(t *testing.T) {
	// 测试 []types.ResponsesItem 类型的直接处理
	items := []types.ResponsesItem{
		{
			Type:    "message",
			Role:    "assistant",
			Content: "Hello, I am doing well!",
		},
		{
			Type: "text",
			Content: []types.ContentBlock{
				{Type: "output_text", Text: "This is output text"},
			},
		},
	}

	result := EstimateResponsesOutputTokens(items)
	if result < 5 {
		t.Errorf("EstimateResponsesOutputTokens([]types.ResponsesItem) = %d, want >= 5", result)
	}
}

func TestEstimateRequestTokens(t *testing.T) {
	tests := []struct {
		name        string
		request     map[string]interface{}
		minExpected int
	}{
		{
			name: "messages_api_request",
			request: map[string]interface{}{
				"model":  "claude-3",
				"system": "You are a helpful assistant.",
				"messages": []interface{}{
					map[string]interface{}{
						"role":    "user",
						"content": "Hello!",
					},
				},
			},
			minExpected: 8,
		},
		{
			name: "with_system_array",
			request: map[string]interface{}{
				"model": "claude-3",
				"system": []interface{}{
					map[string]interface{}{
						"type": "text",
						"text": "You are helpful.",
					},
				},
			},
			minExpected: 4,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			bodyBytes, _ := json.Marshal(tt.request)
			result := EstimateRequestTokens(bodyBytes)
			if result < tt.minExpected {
				t.Errorf("EstimateRequestTokens() = %d, want >= %d", result, tt.minExpected)
			}
		})
	}
}
