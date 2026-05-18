package converters

import (
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/stretchr/testify/assert"
)

// TestGeminiToClaudeRequest_FunctionCall 测试 Gemini function_call 转 Claude tool_use
func TestGeminiToClaudeRequest_FunctionCall(t *testing.T) {
	geminiReq := &types.GeminiRequest{
		Contents: []types.GeminiContent{
			{
				Role: "user",
				Parts: []types.GeminiPart{
					{Text: "What's the weather in Tokyo?"},
				},
			},
			{
				Role: "model",
				Parts: []types.GeminiPart{
					{
						FunctionCall: &types.GeminiFunctionCall{
							Name: "get_weather",
							Args: map[string]interface{}{
								"location": "Tokyo",
								"unit":     "celsius",
							},
						},
					},
				},
			},
		},
		Tools: []types.GeminiTool{
			{
				FunctionDeclarations: []types.GeminiFunctionDeclaration{
					{
						Name:        "get_weather",
						Description: "Get weather information",
						Parameters: map[string]interface{}{
							"type": "object",
							"properties": map[string]interface{}{
								"location": map[string]interface{}{"type": "string"},
								"unit":     map[string]interface{}{"type": "string"},
							},
						},
					},
				},
			},
		},
	}

	claudeReq, err := GeminiToClaudeRequest(geminiReq, "claude-3-5-sonnet-20241022")
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
	userContent, ok := messages[0]["content"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, userContent, 1)
	assert.Equal(t, "text", userContent[0]["type"])
	assert.Equal(t, "What's the weather in Tokyo?", userContent[0]["text"])

	// 验证第二条消息 (assistant with tool_use)
	assert.Equal(t, "assistant", messages[1]["role"])
	assistantContent, ok := messages[1]["content"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, assistantContent, 1)
	assert.Equal(t, "tool_use", assistantContent[0]["type"])
	assert.Equal(t, "get_weather", assistantContent[0]["name"])

	// 验证 tool_use input
	input, ok := assistantContent[0]["input"].(map[string]interface{})
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

// TestGeminiToClaudeRequest_FunctionResponse 测试 Gemini function_response 转 Claude tool_result
func TestGeminiToClaudeRequest_FunctionResponse(t *testing.T) {
	geminiReq := &types.GeminiRequest{
		Contents: []types.GeminiContent{
			{
				Role: "user",
				Parts: []types.GeminiPart{
					{
						FunctionResponse: &types.GeminiFunctionResponse{
							Name: "get_weather",
							Response: map[string]interface{}{
								"temperature": 22,
								"condition":   "sunny",
							},
						},
					},
				},
			},
		},
	}

	claudeReq, err := GeminiToClaudeRequest(geminiReq, "claude-3-5-sonnet-20241022")
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
	assert.Equal(t, "tool_result", content[0]["type"])
	assert.Equal(t, "get_weather", content[0]["tool_use_id"])

	// 验证 tool_result content
	resultContent, ok := content[0]["content"].(map[string]interface{})
	assert.True(t, ok)
	assert.Equal(t, 22, resultContent["temperature"])
	assert.Equal(t, "sunny", resultContent["condition"])
}

// TestGeminiToOpenAIRequest_FunctionCall 测试 Gemini function_call 转 OpenAI tool_calls
func TestGeminiToOpenAIRequest_FunctionCall(t *testing.T) {
	geminiReq := &types.GeminiRequest{
		Contents: []types.GeminiContent{
			{
				Role: "user",
				Parts: []types.GeminiPart{
					{Text: "Search for Go tutorials"},
				},
			},
			{
				Role: "model",
				Parts: []types.GeminiPart{
					{
						FunctionCall: &types.GeminiFunctionCall{
							Name: "web_search",
							Args: map[string]interface{}{
								"query": "golang tutorials",
							},
						},
					},
				},
			},
		},
		Tools: []types.GeminiTool{
			{
				FunctionDeclarations: []types.GeminiFunctionDeclaration{
					{
						Name:        "web_search",
						Description: "Search the web",
						Parameters: map[string]interface{}{
							"type": "object",
							"properties": map[string]interface{}{
								"query": map[string]interface{}{"type": "string"},
							},
						},
					},
				},
			},
		},
	}

	openaiReq, err := GeminiToOpenAIRequest(geminiReq, "gpt-4o")
	assert.NoError(t, err)
	assert.NotNil(t, openaiReq)

	// 验证 model
	assert.Equal(t, "gpt-4o", openaiReq["model"])

	// 验证 messages
	messages, ok := openaiReq["messages"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, messages, 2)

	// 验证第一条消息 (user)
	assert.Equal(t, "user", messages[0]["role"])
	assert.Equal(t, "Search for Go tutorials", messages[0]["content"])

	// 验证第二条消息 (assistant with tool_calls)
	assert.Equal(t, "assistant", messages[1]["role"])
	toolCalls, ok := messages[1]["tool_calls"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, toolCalls, 1)
	assert.Equal(t, "function", toolCalls[0]["type"])

	// 验证 function
	function, ok := toolCalls[0]["function"].(map[string]interface{})
	assert.True(t, ok)
	assert.Equal(t, "web_search", function["name"])

	// 验证 arguments (应该是 JSON 字符串)
	argsStr, ok := function["arguments"].(string)
	assert.True(t, ok)
	assert.Contains(t, argsStr, "golang tutorials")

	// 验证 tools 转换
	tools, ok := openaiReq["tools"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, tools, 1)
	assert.Equal(t, "function", tools[0]["type"])
}

// TestGeminiToOpenAIRequest_FunctionResponse 测试 Gemini function_response 转 OpenAI tool message
func TestGeminiToOpenAIRequest_FunctionResponse(t *testing.T) {
	geminiReq := &types.GeminiRequest{
		Contents: []types.GeminiContent{
			{
				Role: "user",
				Parts: []types.GeminiPart{
					{
						FunctionResponse: &types.GeminiFunctionResponse{
							Name: "web_search",
							Response: map[string]interface{}{
								"results": []string{"Tutorial 1", "Tutorial 2"},
							},
						},
					},
				},
			},
		},
	}

	openaiReq, err := GeminiToOpenAIRequest(geminiReq, "gpt-4o")
	assert.NoError(t, err)
	assert.NotNil(t, openaiReq)

	// 验证 messages
	messages, ok := openaiReq["messages"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, messages, 1)

	// 验证 tool message
	assert.Equal(t, "tool", messages[0]["role"])
	assert.Equal(t, "web_search", messages[0]["tool_call_id"])

	// 验证 content (应该是 JSON 字符串)
	content, ok := messages[0]["content"].(string)
	assert.True(t, ok)
	assert.Contains(t, content, "Tutorial 1")
	assert.Contains(t, content, "Tutorial 2")
}

// TestGeminiToClaudeRequest_MultipleFunctionCalls 测试多个工具调用
func TestGeminiToClaudeRequest_MultipleFunctionCalls(t *testing.T) {
	geminiReq := &types.GeminiRequest{
		Contents: []types.GeminiContent{
			{
				Role: "model",
				Parts: []types.GeminiPart{
					{
						FunctionCall: &types.GeminiFunctionCall{
							Name: "get_weather",
							Args: map[string]interface{}{"location": "Tokyo"},
						},
					},
					{
						FunctionCall: &types.GeminiFunctionCall{
							Name: "get_time",
							Args: map[string]interface{}{"timezone": "Asia/Tokyo"},
						},
					},
				},
			},
		},
	}

	claudeReq, err := GeminiToClaudeRequest(geminiReq, "claude-3-5-sonnet-20241022")
	assert.NoError(t, err)

	messages, ok := claudeReq["messages"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, messages, 1)

	// 验证包含两个 tool_use
	content, ok := messages[0]["content"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, content, 2)
	assert.Equal(t, "tool_use", content[0]["type"])
	assert.Equal(t, "get_weather", content[0]["name"])
	assert.Equal(t, "tool_use", content[1]["type"])
	assert.Equal(t, "get_time", content[1]["name"])
}

// TestGeminiToClaudeRequest_MixedContent 测试混合内容 (text + function_call)
func TestGeminiToClaudeRequest_MixedContent(t *testing.T) {
	geminiReq := &types.GeminiRequest{
		Contents: []types.GeminiContent{
			{
				Role: "model",
				Parts: []types.GeminiPart{
					{Text: "Let me check the weather for you."},
					{
						FunctionCall: &types.GeminiFunctionCall{
							Name: "get_weather",
							Args: map[string]interface{}{"location": "Tokyo"},
						},
					},
				},
			},
		},
	}

	claudeReq, err := GeminiToClaudeRequest(geminiReq, "claude-3-5-sonnet-20241022")
	assert.NoError(t, err)

	messages, ok := claudeReq["messages"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, messages, 1)

	// 验证包含 text 和 tool_use
	content, ok := messages[0]["content"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, content, 2)
	assert.Equal(t, "text", content[0]["type"])
	assert.Equal(t, "Let me check the weather for you.", content[0]["text"])
	assert.Equal(t, "tool_use", content[1]["type"])
	assert.Equal(t, "get_weather", content[1]["name"])
}
