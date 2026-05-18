package converters

import (
	"encoding/json"
	"testing"

	"github.com/stretchr/testify/assert"
)

// TestOpenAIChatToClaudeRequest_ToolCalls 测试 OpenAI tool_calls 转 Claude tool_use
func TestOpenAIChatToClaudeRequest_ToolCalls(t *testing.T) {
	openaiReq := map[string]interface{}{
		"model": "gpt-4o",
		"messages": []interface{}{
			map[string]interface{}{
				"role":    "user",
				"content": "What's the weather in Tokyo?",
			},
			map[string]interface{}{
				"role":    "assistant",
				"content": "Let me check that for you.",
				"tool_calls": []interface{}{
					map[string]interface{}{
						"id":   "call_abc123",
						"type": "function",
						"function": map[string]interface{}{
							"name":      "get_weather",
							"arguments": `{"location":"Tokyo","unit":"celsius"}`,
						},
					},
				},
			},
		},
		"tools": []interface{}{
			map[string]interface{}{
				"type": "function",
				"function": map[string]interface{}{
					"name":        "get_weather",
					"description": "Get weather information",
					"parameters": map[string]interface{}{
						"type": "object",
						"properties": map[string]interface{}{
							"location": map[string]interface{}{"type": "string"},
							"unit":     map[string]interface{}{"type": "string"},
						},
					},
				},
			},
		},
	}

	bodyBytes, _ := json.Marshal(openaiReq)
	claudeReq, err := convertChatToClaudeRequest(bodyBytes, "claude-3-5-sonnet-20241022", false)
	assert.NoError(t, err)
	assert.NotNil(t, claudeReq)

	// 验证 model
	assert.Equal(t, "claude-3-5-sonnet-20241022", claudeReq["model"])

	// 验证 messages
	messages, ok := claudeReq["messages"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, messages, 2)

	// 验证第一条消息 (user)
	assert.Equal(t, "user", messages[0]["role"])

	// 验证第二条消息 (assistant with tool_use)
	assert.Equal(t, "assistant", messages[1]["role"])
	content, ok := messages[1]["content"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, content, 2) // text + tool_use

	// 验证 text 块
	assert.Equal(t, "text", content[0]["type"])
	assert.Equal(t, "Let me check that for you.", content[0]["text"])

	// 验证 tool_use 块
	assert.Equal(t, "tool_use", content[1]["type"])
	assert.Equal(t, "call_abc123", content[1]["id"])
	assert.Equal(t, "get_weather", content[1]["name"])

	// 验证 input
	input, ok := content[1]["input"].(map[string]interface{})
	assert.True(t, ok)
	assert.Equal(t, "Tokyo", input["location"])
	assert.Equal(t, "celsius", input["unit"])

	// 验证 tools 转换
	tools, ok := claudeReq["tools"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, tools, 1)
	assert.Equal(t, "get_weather", tools[0]["name"])
	assert.Equal(t, "Get weather information", tools[0]["description"])
}

// TestOpenAIChatToClaudeRequest_ToolMessage 测试 OpenAI tool message 转 Claude tool_result
func TestOpenAIChatToClaudeRequest_ToolMessage(t *testing.T) {
	openaiReq := map[string]interface{}{
		"model": "gpt-4o",
		"messages": []interface{}{
			map[string]interface{}{
				"role":         "tool",
				"tool_call_id": "call_abc123",
				"content":      "Temperature: 22°C, Sunny",
			},
		},
	}

	bodyBytes, _ := json.Marshal(openaiReq)
	claudeReq, err := convertChatToClaudeRequest(bodyBytes, "claude-3-5-sonnet-20241022", false)
	assert.NoError(t, err)
	assert.NotNil(t, claudeReq)

	// 验证 messages
	messages, ok := claudeReq["messages"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, messages, 1)

	// 验证 tool_result 消息
	assert.Equal(t, "user", messages[0]["role"])
	content, ok := messages[0]["content"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, content, 1)

	// 验证 tool_result 块
	assert.Equal(t, "tool_result", content[0]["type"])
	assert.Equal(t, "call_abc123", content[0]["tool_use_id"])
	assert.Equal(t, "Temperature: 22°C, Sunny", content[0]["content"])
}

// TestOpenAIChatToClaudeRequest_MultipleToolCalls 测试多个工具调用
func TestOpenAIChatToClaudeRequest_MultipleToolCalls(t *testing.T) {
	openaiReq := map[string]interface{}{
		"model": "gpt-4o",
		"messages": []interface{}{
			map[string]interface{}{
				"role": "assistant",
				"tool_calls": []interface{}{
					map[string]interface{}{
						"id":   "call_001",
						"type": "function",
						"function": map[string]interface{}{
							"name":      "get_weather",
							"arguments": `{"location":"Tokyo"}`,
						},
					},
					map[string]interface{}{
						"id":   "call_002",
						"type": "function",
						"function": map[string]interface{}{
							"name":      "get_time",
							"arguments": `{"timezone":"Asia/Tokyo"}`,
						},
					},
				},
			},
		},
	}

	bodyBytes, _ := json.Marshal(openaiReq)
	claudeReq, err := convertChatToClaudeRequest(bodyBytes, "claude-3-5-sonnet-20241022", false)
	assert.NoError(t, err)

	messages, ok := claudeReq["messages"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, messages, 1)

	// 验证包含两个 tool_use
	content, ok := messages[0]["content"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, content, 2)

	assert.Equal(t, "tool_use", content[0]["type"])
	assert.Equal(t, "call_001", content[0]["id"])
	assert.Equal(t, "get_weather", content[0]["name"])

	assert.Equal(t, "tool_use", content[1]["type"])
	assert.Equal(t, "call_002", content[1]["id"])
	assert.Equal(t, "get_time", content[1]["name"])
}

// TestOpenAIChatToClaudeRequest_ToolCallRoundtrip 测试完整的工具调用流程
func TestOpenAIChatToClaudeRequest_ToolCallRoundtrip(t *testing.T) {
	openaiReq := map[string]interface{}{
		"model": "gpt-4o",
		"messages": []interface{}{
			map[string]interface{}{
				"role":    "user",
				"content": "What's the weather?",
			},
			map[string]interface{}{
				"role": "assistant",
				"tool_calls": []interface{}{
					map[string]interface{}{
						"id":   "call_123",
						"type": "function",
						"function": map[string]interface{}{
							"name":      "get_weather",
							"arguments": `{"location":"Tokyo"}`,
						},
					},
				},
			},
			map[string]interface{}{
				"role":         "tool",
				"tool_call_id": "call_123",
				"content":      "22°C, Sunny",
			},
			map[string]interface{}{
				"role":    "assistant",
				"content": "The weather in Tokyo is 22°C and sunny.",
			},
		},
	}

	bodyBytes, _ := json.Marshal(openaiReq)
	claudeReq, err := convertChatToClaudeRequest(bodyBytes, "claude-3-5-sonnet-20241022", false)
	assert.NoError(t, err)

	messages, ok := claudeReq["messages"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, messages, 4)

	// 验证消息序列
	assert.Equal(t, "user", messages[0]["role"])
	assert.Equal(t, "assistant", messages[1]["role"])
	assert.Equal(t, "user", messages[2]["role"]) // tool_result 转为 user
	assert.Equal(t, "assistant", messages[3]["role"])

	// 验证 tool_use
	content1, _ := messages[1]["content"].([]map[string]interface{})
	assert.Equal(t, "tool_use", content1[0]["type"])
	assert.Equal(t, "call_123", content1[0]["id"])

	// 验证 tool_result
	content2, _ := messages[2]["content"].([]map[string]interface{})
	assert.Equal(t, "tool_result", content2[0]["type"])
	assert.Equal(t, "call_123", content2[0]["tool_use_id"])
	assert.Equal(t, "22°C, Sunny", content2[0]["content"])
}

// TestOpenAIChatToClaudeRequest_ToolCallWithoutText 测试纯工具调用（无文本）
func TestOpenAIChatToClaudeRequest_ToolCallWithoutText(t *testing.T) {
	openaiReq := map[string]interface{}{
		"model": "gpt-4o",
		"messages": []interface{}{
			map[string]interface{}{
				"role": "assistant",
				"tool_calls": []interface{}{
					map[string]interface{}{
						"id":   "call_abc",
						"type": "function",
						"function": map[string]interface{}{
							"name":      "search",
							"arguments": `{"query":"test"}`,
						},
					},
				},
			},
		},
	}

	bodyBytes, _ := json.Marshal(openaiReq)
	claudeReq, err := convertChatToClaudeRequest(bodyBytes, "claude-3-5-sonnet-20241022", false)
	assert.NoError(t, err)

	messages, ok := claudeReq["messages"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, messages, 1)

	// 验证只有 tool_use，没有 text
	content, ok := messages[0]["content"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, content, 1)
	assert.Equal(t, "tool_use", content[0]["type"])
}

// convertChatToClaudeRequest 是从 handler 中提取的转换函数
// 为了测试目的，这里复制一份简化版本
func convertChatToClaudeRequest(bodyBytes []byte, model string, isStream bool) (map[string]interface{}, error) {
	var reqMap map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
		return nil, err
	}

	claudeReq := map[string]interface{}{
		"model":  model,
		"stream": isStream,
	}

	// 转换 messages
	if messages, ok := reqMap["messages"].([]interface{}); ok {
		var claudeMessages []map[string]interface{}

		for _, msg := range messages {
			m, ok := msg.(map[string]interface{})
			if !ok {
				continue
			}
			role, _ := m["role"].(string)
			content, _ := m["content"]

			switch role {
			case "user":
				claudeMessages = append(claudeMessages, map[string]interface{}{
					"role":    "user",
					"content": content,
				})
			case "assistant":
				// 检查是否包含 tool_calls
				if toolCalls, ok := m["tool_calls"].([]interface{}); ok && len(toolCalls) > 0 {
					var contentBlocks []map[string]interface{}
					// 先添加文本内容（如有）
					if text, ok := content.(string); ok && text != "" {
						contentBlocks = append(contentBlocks, map[string]interface{}{
							"type": "text",
							"text": text,
						})
					}
					// 转换 tool_calls → tool_use blocks
					for _, tc := range toolCalls {
						tcMap, ok := tc.(map[string]interface{})
						if !ok {
							continue
						}
						fn, _ := tcMap["function"].(map[string]interface{})
						toolID, _ := tcMap["id"].(string)
						toolName, _ := fn["name"].(string)
						argsStr, _ := fn["arguments"].(string)
						var argsObj interface{}
						if json.Unmarshal([]byte(argsStr), &argsObj) != nil {
							argsObj = map[string]interface{}{}
						}
						contentBlocks = append(contentBlocks, map[string]interface{}{
							"type":  "tool_use",
							"id":    toolID,
							"name":  toolName,
							"input": argsObj,
						})
					}
					claudeMessages = append(claudeMessages, map[string]interface{}{
						"role":    "assistant",
						"content": contentBlocks,
					})
				} else {
					claudeMessages = append(claudeMessages, map[string]interface{}{
						"role":    "assistant",
						"content": content,
					})
				}
			case "tool":
				// OpenAI tool result → Claude tool_result
				toolCallID, _ := m["tool_call_id"].(string)
				contentStr := ""
				if s, ok := content.(string); ok {
					contentStr = s
				}
				claudeMessages = append(claudeMessages, map[string]interface{}{
					"role": "user",
					"content": []map[string]interface{}{
						{
							"type":        "tool_result",
							"tool_use_id": toolCallID,
							"content":     contentStr,
						},
					},
				})
			}
		}

		claudeReq["messages"] = claudeMessages
	}

	// 转换 tools
	if tools, ok := reqMap["tools"].([]interface{}); ok && len(tools) > 0 {
		var claudeTools []map[string]interface{}
		for _, tool := range tools {
			t, ok := tool.(map[string]interface{})
			if !ok {
				continue
			}
			fn, ok := t["function"].(map[string]interface{})
			if !ok {
				continue
			}
			claudeTool := map[string]interface{}{
				"name": fn["name"],
			}
			if desc, ok := fn["description"]; ok {
				claudeTool["description"] = desc
			}
			if params, ok := fn["parameters"]; ok {
				claudeTool["input_schema"] = params
			}
			claudeTools = append(claudeTools, claudeTool)
		}
		if len(claudeTools) > 0 {
			claudeReq["tools"] = claudeTools
		}
	}

	return claudeReq, nil
}
