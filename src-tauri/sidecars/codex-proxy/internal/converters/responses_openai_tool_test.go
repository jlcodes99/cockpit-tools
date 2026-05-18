package converters

import (
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/stretchr/testify/assert"
)

// TestResponsesToOpenAIChatMessages_ToolCall 测试 Responses tool_call 转 OpenAI tool_calls
func TestResponsesToOpenAIChatMessages_ToolCall(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	// 模拟 Responses tool_call
	newInput := []interface{}{
		map[string]interface{}{
			"type":    "text",
			"content": "Search for Go tutorials",
		},
		map[string]interface{}{
			"type": "tool_call",
			"tool_use": map[string]interface{}{
				"id":   "toolu_abc123",
				"name": "web_search",
				"input": map[string]interface{}{
					"query": "golang tutorials",
				},
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, newInput, "")
	assert.NoError(t, err)
	assert.Len(t, messages, 2)

	// 验证第一条消息 (user text)
	assert.Equal(t, "user", messages[0]["role"])
	assert.Equal(t, "Search for Go tutorials", messages[0]["content"])

	// 验证第二条消息 (assistant with tool_calls)
	assert.Equal(t, "assistant", messages[1]["role"])

	// Responses tool_call 应该转换为 OpenAI tool_calls
	toolCalls, ok := messages[1]["tool_calls"].([]map[string]interface{})
	assert.True(t, ok, "应该包含 tool_calls 字段")
	assert.Len(t, toolCalls, 1)

	assert.Equal(t, "toolu_abc123", toolCalls[0]["id"])
	assert.Equal(t, "function", toolCalls[0]["type"])

	function, ok := toolCalls[0]["function"].(map[string]interface{})
	assert.True(t, ok)
	assert.Equal(t, "web_search", function["name"])

	// arguments 应该是 JSON 字符串
	argsStr, ok := function["arguments"].(string)
	assert.True(t, ok)
	assert.Contains(t, argsStr, "golang tutorials")
}

// TestResponsesToOpenAIChatMessages_ToolResult 测试 Responses tool_result 转 OpenAI tool message
func TestResponsesToOpenAIChatMessages_ToolResult(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{
			{
				Type:      "function_call",
				CallID:    "toolu_abc123",
				Name:      "web_search",
				Arguments: `{"query":"golang tutorials"}`,
			},
		},
	}

	// 模拟 Responses tool_result
	newInput := []interface{}{
		map[string]interface{}{
			"type": "tool_result",
			"content": map[string]interface{}{
				"tool_use_id": "toolu_abc123",
				"content":     "Found 10 tutorials",
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, newInput, "")
	assert.NoError(t, err)
	assert.Len(t, messages, 2)

	// 验证 tool message
	assert.Equal(t, "tool", messages[1]["role"])
	assert.Equal(t, "toolu_abc123", messages[1]["tool_call_id"])
	assert.Equal(t, "Found 10 tutorials", messages[1]["content"])
}

// TestResponsesToOpenAIChatMessages_ToolResult_ObjectContent 测试对象类型的 tool_result
func TestResponsesToOpenAIChatMessages_ToolResult_ObjectContent(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{
			{
				Type:      "function_call",
				CallID:    "toolu_xyz789",
				Name:      "web_search",
				Arguments: `{"query":"golang tutorials"}`,
			},
		},
	}

	// 模拟对象类型的 tool_result
	newInput := []interface{}{
		map[string]interface{}{
			"type": "tool_result",
			"content": map[string]interface{}{
				"tool_use_id": "toolu_xyz789",
				"content": map[string]interface{}{
					"results": []string{"Tutorial 1", "Tutorial 2"},
					"count":   2,
				},
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, newInput, "")
	assert.NoError(t, err)
	assert.Len(t, messages, 2)

	// 验证 tool message
	assert.Equal(t, "tool", messages[1]["role"])
	assert.Equal(t, "toolu_xyz789", messages[1]["tool_call_id"])

	// content 应该是 JSON 字符串
	content, ok := messages[1]["content"].(string)
	assert.True(t, ok)
	assert.Contains(t, content, "Tutorial 1")
	assert.Contains(t, content, "Tutorial 2")
}

// TestResponsesToOpenAIChatMessages_MultipleToolCalls 测试多个工具调用
func TestResponsesToOpenAIChatMessages_MultipleToolCalls(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	newInput := []interface{}{
		map[string]interface{}{
			"type": "tool_call",
			"tool_use": map[string]interface{}{
				"id":   "toolu_001",
				"name": "get_weather",
				"input": map[string]interface{}{
					"location": "Tokyo",
				},
			},
		},
		map[string]interface{}{
			"type": "tool_call",
			"tool_use": map[string]interface{}{
				"id":   "toolu_002",
				"name": "get_time",
				"input": map[string]interface{}{
					"timezone": "Asia/Tokyo",
				},
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, newInput, "")
	assert.NoError(t, err)

	// 连续 function_call 应合并为一条 assistant 消息
	assert.Len(t, messages, 1)
	assert.Equal(t, "assistant", messages[0]["role"])

	// 该消息应包含两个 tool_call
	toolCalls, ok := messages[0]["tool_calls"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, toolCalls, 2)
	assert.Equal(t, "get_weather", toolCalls[0]["function"].(map[string]interface{})["name"])
	assert.Equal(t, "get_time", toolCalls[1]["function"].(map[string]interface{})["name"])
}

// TestResponsesToOpenAIChatMessages_ToolCallRoundtrip 测试完整的工具调用流程
func TestResponsesToOpenAIChatMessages_ToolCallRoundtrip(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	// 模拟完整的工具调用流程
	newInput := []interface{}{
		map[string]interface{}{
			"type":    "text",
			"content": "What's the weather?",
		},
		map[string]interface{}{
			"type": "tool_call",
			"tool_use": map[string]interface{}{
				"id":   "toolu_123",
				"name": "get_weather",
				"input": map[string]interface{}{
					"location": "Tokyo",
				},
			},
		},
		map[string]interface{}{
			"type": "tool_result",
			"content": map[string]interface{}{
				"tool_use_id": "toolu_123",
				"content":     "22°C, Sunny",
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, newInput, "")
	assert.NoError(t, err)
	assert.Len(t, messages, 3)

	// 验证消息序列
	assert.Equal(t, "user", messages[0]["role"])
	assert.Equal(t, "assistant", messages[1]["role"])
	assert.Equal(t, "tool", messages[2]["role"])

	// 验证 tool_call_id 匹配
	toolCalls, _ := messages[1]["tool_calls"].([]map[string]interface{})
	assert.Equal(t, "toolu_123", toolCalls[0]["id"])
	assert.Equal(t, "toolu_123", messages[2]["tool_call_id"])
}

func TestResponsesToOpenAIChatMessages_SkipsLegacyToolCallMissingToolUse(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"type":    "text",
			"content": "hello",
		},
		map[string]interface{}{
			"type": "tool_call",
		},
	}, "")
	assert.NoError(t, err)
	assert.Len(t, messages, 1)
	assert.Equal(t, "user", messages[0]["role"])
	assert.Equal(t, "hello", messages[0]["content"])
}

func TestResponsesToOpenAIChatMessages_FunctionCallOutputMissingCallIDReturnsNilMessage(t *testing.T) {
	msg := responsesItemToOpenAIMessage(types.ResponsesItem{
		Type:   "function_call_output",
		Output: "Sunny",
	})
	assert.Nil(t, msg)
}

func TestResponsesToOpenAIChatMessages_FunctionCallMissingNameReturnsNilMessage(t *testing.T) {
	msg := responsesItemToOpenAIMessage(types.ResponsesItem{
		Type:      "function_call",
		CallID:    "call_1",
		Arguments: `{"location":"Tokyo"}`,
	})
	assert.Nil(t, msg)
}

func TestResponsesToOpenAIChatMessages_FunctionCallOutputObjectContentSerializesJSON(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{
			{
				Type:      "function_call",
				CallID:    "call_1",
				Name:      "get_weather",
				Arguments: `{"location":"Tokyo"}`,
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"type":    "function_call_output",
			"call_id": "call_1",
			"output": map[string]interface{}{
				"temperature": 72,
				"condition":   "sunny",
			},
		},
	}, "")
	assert.NoError(t, err)
	assert.Len(t, messages, 2)
	assert.Equal(t, "tool", messages[1]["role"])
	content, ok := messages[1]["content"].(string)
	assert.True(t, ok)
	assert.Contains(t, content, "temperature")
	assert.Contains(t, content, "sunny")
}

func TestResponsesToOpenAIChatMessages_MergesReasoningIntoFollowingAssistantMessage(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"type":   "reasoning",
			"status": "completed",
			"summary": []interface{}{
				map[string]interface{}{"type": "summary_text", "text": "previous reasoning"},
			},
		},
		map[string]interface{}{
			"type": "message",
			"role": "assistant",
			"content": []interface{}{
				map[string]interface{}{"type": "output_text", "text": "previous text"},
			},
		},
	}, "")

	assert.NoError(t, err)
	assert.Len(t, messages, 1)
	assert.Equal(t, "assistant", messages[0]["role"])
	assert.Equal(t, "previous reasoning", messages[0]["reasoning_content"])
	assertOpenAIChatTextContent(t, "previous text", messages[0]["content"])
}

func TestResponsesToOpenAIChatMessages_MultipleFunctionCallsMergeIntoOneMessage(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"type":      "function_call",
			"call_id":   "call_1",
			"name":      "get_weather",
			"arguments": `{"location":"Tokyo"}`,
		},
		map[string]interface{}{
			"type":      "function_call",
			"call_id":   "call_2",
			"name":      "get_time",
			"arguments": `{"timezone":"UTC"}`,
		},
	}, "")
	assert.NoError(t, err)
	// 连续 function_call 合并为单条 assistant 消息
	assert.Len(t, messages, 1)
	assert.Equal(t, "assistant", messages[0]["role"])
	toolCalls, ok := messages[0]["tool_calls"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, toolCalls, 2)
	assert.Equal(t, "call_1", toolCalls[0]["id"])
	assert.Equal(t, "call_2", toolCalls[1]["id"])
}

func TestResponsesToOpenAIChatMessages_NormalizesLegacySessionMessages(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{
			{
				Type: "tool_call",
				ToolUse: &types.ToolUse{
					ID:   "toolu_hist",
					Name: "get_weather",
					Input: map[string]interface{}{
						"location": "Berlin",
					},
				},
			},
			{
				Type: "tool_result",
				Content: map[string]interface{}{
					"tool_use_id": "toolu_hist",
					"content":     map[string]interface{}{"temperature": 18},
				},
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{}, "")
	assert.NoError(t, err)
	assert.Len(t, messages, 2)
	assert.Equal(t, "assistant", messages[0]["role"])
	assert.Equal(t, "tool", messages[1]["role"])

	toolCalls, ok := messages[0]["tool_calls"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, toolCalls, 1)
	assert.Equal(t, "toolu_hist", toolCalls[0]["id"])
	assert.Equal(t, "get_weather", toolCalls[0]["function"].(map[string]interface{})["name"])
	assert.Equal(t, "toolu_hist", messages[1]["tool_call_id"])
}

func TestResponsesToOpenAIChatMessages_FunctionCallDefaultsCallIDToName(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"type":      "function_call",
			"name":      "get_weather",
			"arguments": `{"location":"Tokyo"}`,
		},
	}, "")
	assert.NoError(t, err)
	assert.Len(t, messages, 1)

	toolCalls, ok := messages[0]["tool_calls"].([]map[string]interface{})
	assert.True(t, ok)
	assert.Len(t, toolCalls, 1)
	assert.Equal(t, "get_weather", toolCalls[0]["id"])
}

func TestResponsesToOpenAIChatMessages_ReordersUserMessagesAfterPendingToolOutputs(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{
			{
				Type:      "function_call",
				CallID:    "call_1",
				Name:      "exec_command",
				Arguments: `{"cmd":"go test ./..."}`,
			},
			{
				Type:      "function_call",
				CallID:    "call_2",
				Name:      "exec_command",
				Arguments: `{"cmd":"bun run build"}`,
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"type": "message",
			"role": "user",
			"content": []interface{}{
				map[string]interface{}{
					"type": "input_text",
					"text": "Approved command prefix saved",
				},
			},
		},
		map[string]interface{}{
			"type":    "function_call_output",
			"call_id": "call_1",
			"output":  "tests passed",
		},
		map[string]interface{}{
			"type":    "function_call_output",
			"call_id": "call_2",
			"output":  "build passed",
		},
	}, "")

	assert.NoError(t, err)
	assert.Len(t, messages, 4)
	assert.Equal(t, "assistant", messages[0]["role"])
	assert.Equal(t, "tool", messages[1]["role"])
	assert.Equal(t, "tool", messages[2]["role"])
	assert.Equal(t, "user", messages[3]["role"])
	assert.Equal(t, "call_1", messages[1]["tool_call_id"])
	assert.Equal(t, "call_2", messages[2]["tool_call_id"])
	assertOpenAIChatTextContent(t, "Approved command prefix saved", messages[3]["content"])
}

// TestResponsesToOpenAIChatMessages_DeepSeekMultiTurnToolCalls 模拟 DeepSeek 多轮 tool_calls 场景
// 验证 Responses→Chat 转换符合 DeepSeek 文档要求：
// 1. 有 tool_calls 的 assistant 消息必须包含 reasoning_content
// 2. 不产生连续 assistant 消息
// 3. function_call 不重复（session 和 new input 都有时只保留一份）
func TestResponsesToOpenAIChatMessages_DeepSeekMultiTurnToolCalls(t *testing.T) {
	// 模拟 session：第一轮 DeepSeek 响应被 OpenAIChatResponseToResponses 转换后的三个 ResponsesItem
	sess := &session.Session{
		Messages: []types.ResponsesItem{
			// 1. reasoning（来自 reasoning_content）
			{
				Type:   "reasoning",
				Status: "completed",
				Summary: []interface{}{map[string]interface{}{
					"type": "summary_text",
					"text": "I need to run go vet to check the code.",
				}},
			},
			// 2. assistant message（来自 content）
			{
				Type: "message",
				Role: "assistant",
				Content: []types.ContentBlock{{
					Type: "output_text",
					Text: "Let me run go vet.",
				}},
			},
			// 3. function_call（来自 tool_calls）
			{
				Type:      "function_call",
				Status:    "completed",
				CallID:    "call_001",
				Name:      "exec_command",
				Arguments: `{"cmd":"go vet ./..."}`,
			},
		},
	}

	// 模拟第二轮 new input：客户端回传 function_call + function_call_output + 新 user 消息
	newInput := []interface{}{
		map[string]interface{}{
			"type":      "function_call",
			"call_id":   "call_001",
			"name":      "exec_command",
			"arguments": `{"cmd":"go vet ./..."}`,
		},
		map[string]interface{}{
			"type":    "function_call_output",
			"call_id": "call_001",
			"output":  "no issues found",
		},
		map[string]interface{}{
			"type": "message",
			"role": "user",
			"content": []interface{}{
				map[string]interface{}{
					"type": "input_text",
					"text": "run tests now",
				},
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, newInput, "You are a coding agent.")
	assert.NoError(t, err)

	// 期望消息序列：
	// [0] system
	// [1] assistant (content + reasoning_content + tool_calls，合并为一条)
	// [2] tool (tool_call_id=call_001)
	// [3] user (run tests now)
	assert.Len(t, messages, 4, "should have system, assistant, tool, user messages")

	// [0] system
	assert.Equal(t, "system", messages[0]["role"])

	// [1] assistant - 必须同时包含 content、reasoning_content、tool_calls
	assert.Equal(t, "assistant", messages[1]["role"])
	assert.Equal(t, "Let me run go vet.", messages[1]["content"], "assistant should have content")
	assert.Equal(t, "I need to run go vet to check the code.", messages[1]["reasoning_content"],
		"assistant must have reasoning_content for DeepSeek thinking mode with tool_calls")

	tc, ok := messages[1]["tool_calls"].([]map[string]interface{})
	assert.True(t, ok, "assistant should have tool_calls")
	// tool_calls 不应重复（session 和 new input 各有一份，只保留一份）
	assert.Len(t, tc, 1, "tool_calls should not be duplicated")
	assert.Equal(t, "call_001", tc[0]["id"])
	assert.Equal(t, "exec_command", tc[0]["function"].(map[string]interface{})["name"])

	// [2] tool
	assert.Equal(t, "tool", messages[2]["role"])
	assert.Equal(t, "call_001", messages[2]["tool_call_id"])
	assert.Equal(t, "no issues found", messages[2]["content"])

	// [3] user
	assert.Equal(t, "user", messages[3]["role"])
	assertOpenAIChatTextContent(t, "run tests now", messages[3]["content"])
}

func TestResponsesToOpenAIChatMessages_DowngradesOrphanToolOutput(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{
			{
				Type:   "reasoning",
				Status: "completed",
				Summary: []interface{}{map[string]interface{}{
					"type": "summary_text",
					"text": "I need the previous tool result.",
				}},
			},
		},
	}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"type":    "function_call_output",
			"call_id": "missing_call",
			"output":  "tool output without a matching call",
		},
	}, "")

	assert.NoError(t, err)
	assert.Len(t, messages, 2)
	assert.Equal(t, "assistant", messages[0]["role"])
	assert.NotContains(t, messages[0], "tool_calls")
	assert.Equal(t, "user", messages[1]["role"])
	assert.Equal(t, "Function call output (missing_call): tool output without a matching call", messages[1]["content"])
	assert.NotContains(t, messages[1], "tool_call_id")
}

func TestResponsesToOpenAIChatMessages_InferMessageTypeForRoleContentInput(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"role":    "user",
			"content": "Who are you?",
		},
	}, "")

	assert.NoError(t, err)
	assert.Len(t, messages, 1)
	assert.Equal(t, "user", messages[0]["role"])
	assert.Equal(t, "Who are you?", messages[0]["content"])
}

// TestResponsesToOpenAIChatMessages_OrphanReasoningUsesEmptyContent 验证孤儿 reasoning 不会产生 content:null。
func TestResponsesToOpenAIChatMessages_OrphanReasoningUsesEmptyContent(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"type":   "reasoning",
			"status": "completed",
			"summary": []interface{}{
				map[string]interface{}{"type": "summary_text", "text": "partial reasoning"},
			},
		},
		map[string]interface{}{
			"type": "message",
			"role": "user",
			"content": []interface{}{
				map[string]interface{}{"type": "input_text", "text": "继续"},
			},
		},
	}, "")

	assert.NoError(t, err)
	assert.Len(t, messages, 2)
	assert.Equal(t, "assistant", messages[0]["role"])
	assert.Equal(t, "", messages[0]["content"])
	assert.Equal(t, "partial reasoning", messages[0]["reasoning_content"])
	assert.Equal(t, "user", messages[1]["role"])
}

// TestResponsesToOpenAIChatMessages_InterruptedSessionMergesOrphanReasoning 模拟 Codex 停止生成后继续输入。
func TestResponsesToOpenAIChatMessages_InterruptedSessionMergesOrphanReasoning(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{
		{
			Type:   "reasoning",
			Status: "completed",
			Summary: []interface{}{map[string]interface{}{
				"type": "summary_text",
				"text": "first reasoning",
			}},
		},
		{
			Type: "message",
			Role: "assistant",
			Content: []types.ContentBlock{{
				Type: "output_text",
				Text: "partial answer before stop",
			}},
		},
		{
			Type:   "reasoning",
			Status: "completed",
			Summary: []interface{}{map[string]interface{}{
				"type": "summary_text",
				"text": "interrupted reasoning",
			}},
		},
	}}

	messages, err := ResponsesToOpenAIChatMessages(sess, []interface{}{
		map[string]interface{}{
			"type": "message",
			"role": "user",
			"content": []interface{}{
				map[string]interface{}{"type": "input_text", "text": "后边无论输入什么"},
			},
		},
	}, "")

	assert.NoError(t, err)
	assert.Len(t, messages, 2)
	assert.Equal(t, "assistant", messages[0]["role"])
	assert.Equal(t, "partial answer before stop", messages[0]["content"])
	assert.Equal(t, "first reasoning\ninterrupted reasoning", messages[0]["reasoning_content"])
	assert.Equal(t, "user", messages[1]["role"])
}

// TestResponsesToolsToOpenAI_RequiredAutoFill 验证 required 字段自动补齐
// 根因：部分上游（如 duckcoding 的 OpenAI 严格 schema）在 required 缺失时
// 直接按 None 校验，抛 "Invalid schema for function ...: None is not of type 'array'"
func TestResponsesToolsToOpenAI_RequiredAutoFill(t *testing.T) {
	tools := []map[string]interface{}{
		{
			"type":        "function",
			"name":        "list_mcp_resources",
			"description": "Lists resources provided by MCP servers.",
			"parameters": map[string]interface{}{
				"type": "object",
				"properties": map[string]interface{}{
					"cursor": map[string]interface{}{"type": "string"},
					"server": map[string]interface{}{"type": "string"},
				},
				"additionalProperties": false,
			},
		},
	}

	openaiTools := responsesToolsToOpenAI(tools)
	assert.Len(t, openaiTools, 1)

	fn := openaiTools[0]["function"].(map[string]interface{})
	params := fn["parameters"].(map[string]interface{})
	required, ok := params["required"]
	assert.True(t, ok, "parameters.required 必须存在以兼容严格 schema 校验")
	arr, ok := required.([]interface{})
	assert.True(t, ok, "required 必须是 array 类型")
	assert.Equal(t, 0, len(arr))
}

// TestResponsesToolsToOpenAI_SkipsCustomTools 非 function 类型不应透传给 Chat Completions
func TestResponsesToolsToOpenAI_SkipsCustomTools(t *testing.T) {
	tools := []map[string]interface{}{
		{"type": "custom", "name": "apply_patch"},
		{"type": "web_search"},
		{"type": "function", "name": "do_thing", "parameters": map[string]interface{}{"type": "object", "properties": map[string]interface{}{}}},
	}

	openaiTools := responsesToolsToOpenAI(tools)
	assert.Len(t, openaiTools, 1, "仅保留 function 类型工具")

	fn := openaiTools[0]["function"].(map[string]interface{})
	assert.Equal(t, "do_thing", fn["name"])
}

// TestResponsesToolsToOpenAI_EmptyParametersDefault 无 parameters 时使用默认值
func TestResponsesToolsToOpenAI_EmptyParametersDefault(t *testing.T) {
	tools := []map[string]interface{}{
		{"type": "function", "name": "ping"},
	}

	openaiTools := responsesToolsToOpenAI(tools)
	assert.Len(t, openaiTools, 1)

	fn := openaiTools[0]["function"].(map[string]interface{})
	params := fn["parameters"].(map[string]interface{})
	assert.Equal(t, "object", params["type"])
	assert.NotNil(t, params["properties"])
	assert.NotNil(t, params["required"])
}

func assertOpenAIChatTextContent(t *testing.T, expected string, content interface{}) {
	t.Helper()

	switch v := content.(type) {
	case string:
		assert.Equal(t, expected, v)
	case []map[string]interface{}:
		if !assert.Len(t, v, 1) {
			return
		}
		assert.Equal(t, "text", v[0]["type"])
		assert.Equal(t, expected, v[0]["text"])
	case []interface{}:
		if !assert.Len(t, v, 1) {
			return
		}
		part, ok := v[0].(map[string]interface{})
		if !assert.True(t, ok) {
			return
		}
		assert.Equal(t, "text", part["type"])
		assert.Equal(t, expected, part["text"])
	default:
		assert.Failf(t, "unsupported content type", "got %T", content)
	}
}
