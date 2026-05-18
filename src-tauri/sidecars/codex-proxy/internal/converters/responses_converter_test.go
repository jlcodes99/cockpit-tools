package converters

import (
	"encoding/json"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// TestClaudeResponseToResponses_ToolUse 测试 Claude 响应中的 tool_use 转换
func TestClaudeResponseToResponses_ToolUse(t *testing.T) {
	claudeResp := map[string]interface{}{
		"id":    "msg_123",
		"model": "claude-3-5-sonnet-20241022",
		"content": []interface{}{
			map[string]interface{}{
				"type": "text",
				"text": "Let me search for that information.",
			},
			map[string]interface{}{
				"type": "tool_use",
				"id":   "toolu_abc123",
				"name": "web_search",
				"input": map[string]interface{}{
					"query": "golang best practices",
				},
			},
		},
		"usage": map[string]interface{}{
			"input_tokens":  100,
			"output_tokens": 50,
		},
	}

	result, err := ClaudeResponseToResponses(claudeResp, "session_123")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	if len(result.Output) != 2 {
		t.Fatalf("期望 2 个输出项，实际 %d", len(result.Output))
	}
	if result.Output[0].Type != "text" {
		t.Errorf("第一项类型期望 'text'，实际 '%s'", result.Output[0].Type)
	}
	if result.Output[1].Type != "function_call" {
		t.Fatalf("第二项类型期望 'function_call'，实际 '%s'", result.Output[1].Type)
	}
	if result.Output[1].CallID != "toolu_abc123" {
		t.Errorf("call_id 期望 'toolu_abc123'，实际 '%s'", result.Output[1].CallID)
	}
	if result.Output[1].Name != "web_search" {
		t.Errorf("name 期望 'web_search'，实际 '%s'", result.Output[1].Name)
	}

	var args map[string]interface{}
	if err := json.Unmarshal([]byte(result.Output[1].Arguments), &args); err != nil {
		t.Fatalf("arguments 反序列化失败: %v", err)
	}
	if args["query"] != "golang best practices" {
		t.Errorf("arguments.query 不匹配")
	}
}

func TestClaudeResponseToResponses_ToolResult(t *testing.T) {
	claudeResp := map[string]interface{}{
		"id":    "msg_124",
		"model": "claude-3-5-sonnet-20241022",
		"content": []interface{}{
			map[string]interface{}{
				"type":        "tool_result",
				"tool_use_id": "toolu_abc123",
				"content": map[string]interface{}{
					"temperature": 72,
				},
			},
		},
	}

	result, err := ClaudeResponseToResponses(claudeResp, "session_124")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}
	if len(result.Output) != 1 {
		t.Fatalf("期望 1 个输出项，实际 %d", len(result.Output))
	}
	if result.Output[0].Type != "function_call_output" {
		t.Fatalf("类型期望 'function_call_output'，实际 '%s'", result.Output[0].Type)
	}
	if result.Output[0].CallID != "toolu_abc123" {
		t.Fatalf("call_id 期望 'toolu_abc123'，实际 '%s'", result.Output[0].CallID)
	}
	outputMap, ok := result.Output[0].Output.(map[string]interface{})
	if !ok {
		t.Fatalf("output 类型错误: %#v", result.Output[0].Output)
	}
	if outputMap["temperature"] != 72 {
		t.Fatalf("temperature 不匹配: %#v", outputMap)
	}
}

// TestResponsesToClaudeMessages_ToolCall 测试 Responses tool_call 转 Claude
func TestResponsesToClaudeMessages_ToolCall(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	newInput := []interface{}{
		map[string]interface{}{
			"type": "tool_call",
			"tool_use": map[string]interface{}{
				"id":   "toolu_xyz789",
				"name": "get_weather",
				"input": map[string]interface{}{
					"location": "San Francisco",
				},
			},
		},
	}

	messages, _, err := ResponsesToClaudeMessages(sess, newInput, "")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	if len(messages) != 1 {
		t.Fatalf("期望 1 条消息，实际 %d", len(messages))
	}

	// 验证消息角色
	if messages[0].Role != "assistant" {
		t.Errorf("tool_call 角色期望 'assistant'，实际 '%s'", messages[0].Role)
	}

	// 验证内容块
	content, ok := messages[0].Content.([]types.ClaudeContent)
	if !ok {
		t.Fatal("Content 类型错误")
	}
	if len(content) != 1 {
		t.Fatalf("期望 1 个内容块，实际 %d", len(content))
	}

	// 验证 tool_use 内容块
	if content[0].Type != "tool_use" {
		t.Errorf("内容块类型期望 'tool_use'，实际 '%s'", content[0].Type)
	}
	if content[0].ID != "toolu_xyz789" {
		t.Errorf("tool_use.id 期望 'toolu_xyz789'，实际 '%s'", content[0].ID)
	}
	if content[0].Name != "get_weather" {
		t.Errorf("tool_use.name 期望 'get_weather'，实际 '%s'", content[0].Name)
	}

	inputMap, ok := content[0].Input.(map[string]interface{})
	if !ok {
		t.Fatal("tool_use.input 类型错误")
	}
	if inputMap["location"] != "San Francisco" {
		t.Errorf("tool_use.input.location 不匹配")
	}
}

func TestResponsesToClaudeMessages_FunctionCall(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	newInput := []interface{}{
		map[string]interface{}{
			"type":      "function_call",
			"call_id":   "call_123",
			"name":      "get_weather",
			"arguments": `{"location":"NYC"}`,
		},
	}

	messages, _, err := ResponsesToClaudeMessages(sess, newInput, "")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}
	if len(messages) != 1 {
		t.Fatalf("期望 1 条消息，实际 %d", len(messages))
	}
	content := messages[0].Content.([]types.ClaudeContent)
	if content[0].Type != "tool_use" || content[0].ID != "call_123" || content[0].Name != "get_weather" {
		t.Fatalf("function_call 转换结果不正确: %#v", content[0])
	}
}

func TestResponsesToClaudeMessages_MergesReasoningIntoFollowingAssistantMessage(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	messages, _, err := ResponsesToClaudeMessages(sess, []interface{}{
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
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}
	if len(messages) != 1 {
		t.Fatalf("期望 1 条消息，实际 %d", len(messages))
	}
	content, ok := messages[0].Content.([]types.ClaudeContent)
	if !ok {
		t.Fatal("Content 类型错误")
	}
	if len(content) != 2 {
		t.Fatalf("期望 2 个内容块，实际 %d", len(content))
	}
	if content[0].Type != "thinking" || content[0].Thinking != "previous reasoning" {
		t.Fatalf("thinking block 不匹配: %#v", content[0])
	}
	if content[1].Type != "text" || content[1].Text != "previous text" {
		t.Fatalf("text block 不匹配: %#v", content[1])
	}
}

// TestResponsesToClaudeMessages_ToolResult 测试 Responses tool_result 转 Claude
func TestResponsesToClaudeMessages_ToolResult(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	// 测试字符串结果
	newInput := []interface{}{
		map[string]interface{}{
			"type": "tool_result",
			"content": map[string]interface{}{
				"tool_use_id": "toolu_xyz789",
				"content":     "Temperature: 72°F, Sunny",
			},
		},
	}

	messages, _, err := ResponsesToClaudeMessages(sess, newInput, "")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	if len(messages) != 1 {
		t.Fatalf("期望 1 条消息，实际 %d", len(messages))
	}

	// 验证消息角色
	if messages[0].Role != "user" {
		t.Errorf("tool_result 角色期望 'user'，实际 '%s'", messages[0].Role)
	}

	// 验证内容块
	content, ok := messages[0].Content.([]types.ClaudeContent)
	if !ok {
		t.Fatal("Content 类型错误")
	}
	if len(content) != 1 {
		t.Fatalf("期望 1 个内容块，实际 %d", len(content))
	}

	// 验证 tool_result 内容块
	if content[0].Type != "tool_result" {
		t.Errorf("内容块类型期望 'tool_result'，实际 '%s'", content[0].Type)
	}
	if content[0].ToolUseID != "toolu_xyz789" {
		t.Errorf("tool_result.tool_use_id 期望 'toolu_xyz789'，实际 '%s'", content[0].ToolUseID)
	}
	if content[0].Content != "Temperature: 72°F, Sunny" {
		t.Errorf("tool_result.content 不匹配")
	}
}

// TestResponsesToClaudeMessages_ToolResult_ObjectContent 测试对象类型的 tool_result
func TestResponsesToClaudeMessages_ToolResult_ObjectContent(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	// 测试对象结果
	newInput := []interface{}{
		map[string]interface{}{
			"type": "tool_result",
			"content": map[string]interface{}{
				"tool_use_id": "toolu_abc123",
				"content": map[string]interface{}{
					"temperature": 72,
					"condition":   "sunny",
				},
			},
		},
	}

	messages, _, err := ResponsesToClaudeMessages(sess, newInput, "")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	content, ok := messages[0].Content.([]types.ClaudeContent)
	if !ok {
		t.Fatal("Content 类型错误")
	}

	// 验证对象内容
	contentMap, ok := content[0].Content.(map[string]interface{})
	if !ok {
		t.Fatal("tool_result.content 应该是 map[string]interface{}")
	}
	if contentMap["temperature"] != 72 {
		t.Errorf("temperature 不匹配")
	}
	if contentMap["condition"] != "sunny" {
		t.Errorf("condition 不匹配")
	}
}

// TestResponsesToClaudeMessages_RoundtripShape 测试往返转换的结构正确性
func TestResponsesToClaudeMessages_RoundtripShape(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	// 模拟完整的工具调用流程
	newInput := []interface{}{
		map[string]interface{}{
			"type":    "text",
			"content": "Please search for Go tutorials",
		},
		map[string]interface{}{
			"type": "tool_call",
			"tool_use": map[string]interface{}{
				"id":   "toolu_001",
				"name": "web_search",
				"input": map[string]interface{}{
					"query": "golang tutorials",
				},
			},
		},
		map[string]interface{}{
			"type": "tool_result",
			"content": map[string]interface{}{
				"tool_use_id": "toolu_001",
				"content":     "Found 10 tutorials",
			},
		},
	}

	messages, _, err := ResponsesToClaudeMessages(sess, newInput, "")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	// 验证消息数量和角色序列
	if len(messages) != 3 {
		t.Fatalf("期望 3 条消息，实际 %d", len(messages))
	}

	expectedRoles := []string{"user", "assistant", "user"}
	for i, msg := range messages {
		if msg.Role != expectedRoles[i] {
			t.Errorf("消息 %d 角色期望 '%s'，实际 '%s'", i, expectedRoles[i], msg.Role)
		}
	}

	// 验证第一条消息（text）
	content0, ok := messages[0].Content.([]types.ClaudeContent)
	if !ok || len(content0) != 1 || content0[0].Type != "text" {
		t.Error("第一条消息应该是 text 类型")
	}

	// 验证第二条消息（tool_use）
	content1, ok := messages[1].Content.([]types.ClaudeContent)
	if !ok || len(content1) != 1 || content1[0].Type != "tool_use" {
		t.Error("第二条消息应该是 tool_use 类型")
	}

	// 验证第三条消息（tool_result）
	content2, ok := messages[2].Content.([]types.ClaudeContent)
	if !ok || len(content2) != 1 || content2[0].Type != "tool_result" {
		t.Error("第三条消息应该是 tool_result 类型")
	}
}

// TestClaudeResponseToResponses_TextOnlyRegression 回归测试：纯文本场景
func TestClaudeResponseToResponses_TextOnlyRegression(t *testing.T) {
	claudeResp := map[string]interface{}{
		"id":    "msg_456",
		"model": "claude-3-5-sonnet-20241022",
		"content": []interface{}{
			map[string]interface{}{
				"type": "text",
				"text": "Hello, how can I help you?",
			},
		},
		"usage": map[string]interface{}{
			"input_tokens":  50,
			"output_tokens": 20,
		},
	}

	result, err := ClaudeResponseToResponses(claudeResp, "session_456")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	// 验证纯文本场景不受影响
	if len(result.Output) != 1 {
		t.Fatalf("期望 1 个输出项，实际 %d", len(result.Output))
	}

	if result.Output[0].Type != "text" {
		t.Errorf("类型期望 'text'，实际 '%s'", result.Output[0].Type)
	}

	if result.Output[0].Content != "Hello, how can I help you?" {
		t.Errorf("内容不匹配")
	}

	// 验证 usage
	if result.Usage.InputTokens != 50 {
		t.Errorf("input_tokens 期望 50，实际 %d", result.Usage.InputTokens)
	}
	if result.Usage.OutputTokens != 20 {
		t.Errorf("output_tokens 期望 20，实际 %d", result.Usage.OutputTokens)
	}
}

// TestResponsesToClaudeMessages_ToolCallMissingToolUse 测试缺少 tool_use 字段的兼容性处理
func TestResponsesToClaudeMessages_ToolCallMissingToolUse(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	// tool_call 但缺少 tool_use 字段（历史消息场景）
	newInput := []interface{}{
		map[string]interface{}{
			"type": "tool_call",
		},
	}

	messages, _, err := ResponsesToClaudeMessages(sess, newInput, "")
	// 应该跳过而不是报错，保持向后兼容
	if err != nil {
		t.Errorf("不应该返回错误，应该跳过: %v", err)
	}
	// 应该返回空消息列表（跳过了无效的 tool_call）
	if len(messages) != 0 {
		t.Errorf("期望返回 0 条消息（跳过无效 tool_call），实际 %d", len(messages))
	}
}

func TestResponsesToClaudeMessages_SkipsLegacyToolResultForSkippedToolCall(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	newInput := []interface{}{
		map[string]interface{}{
			"type": "tool_call",
			"id":   "toolu_missing",
		},
		map[string]interface{}{
			"type": "tool_result",
			"content": map[string]interface{}{
				"tool_use_id": "toolu_missing",
				"content":     "should be skipped",
			},
		},
	}

	messages, _, err := ResponsesToClaudeMessages(sess, newInput, "")
	if err != nil {
		t.Fatalf("不应该返回错误: %v", err)
	}
	if len(messages) != 0 {
		t.Fatalf("期望跳过配对的 legacy tool_result，实际得到 %d 条消息", len(messages))
	}
}

// TestResponsesToClaudeMessages_ToolResultMissingToolUseID 测试缺少 tool_use_id 的错误处理
func TestResponsesToClaudeMessages_NormalizesLegacySessionMessages(t *testing.T) {
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

	messages, _, err := ResponsesToClaudeMessages(sess, []interface{}{}, "")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}
	if len(messages) != 2 {
		t.Fatalf("期望 2 条历史消息，实际 %d", len(messages))
	}
	if messages[0].Role != "assistant" || messages[1].Role != "user" {
		t.Fatalf("历史消息角色不正确: %#v", messages)
	}
	content0 := messages[0].Content.([]types.ClaudeContent)
	if content0[0].Type != "tool_use" || content0[0].ID != "toolu_hist" || content0[0].Name != "get_weather" {
		t.Fatalf("历史 tool_call 未正确归一化: %#v", content0[0])
	}
	content1 := messages[1].Content.([]types.ClaudeContent)
	if content1[0].Type != "tool_result" || content1[0].ToolUseID != "toolu_hist" {
		t.Fatalf("历史 tool_result 未正确归一化: %#v", content1[0])
	}
}

func TestResponsesToClaudeMessages_FunctionCallMissingName(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	_, _, err := ResponsesToClaudeMessages(sess, []interface{}{
		map[string]interface{}{
			"type":      "function_call",
			"call_id":   "call_1",
			"arguments": `{"location":"Tokyo"}`,
		},
	}, "")
	if err == nil {
		t.Fatal("expected error for missing name")
	}
	if err.Error() != "转换新消息失败: function_call 缺少 name" {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestResponsesToClaudeMessages_FunctionCallOutputMissingCallIDErrorMessageStable(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	_, _, err := ResponsesToClaudeMessages(sess, []interface{}{
		map[string]interface{}{
			"type":   "function_call_output",
			"output": "Sunny",
		},
	}, "")
	if err == nil {
		t.Fatal("expected error for missing call_id")
	}
	if err.Error() != "转换新消息失败: function_call_output 缺少 call_id" {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestResponsesToClaudeMessages_FunctionCallOutputMissingCallID(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	_, _, err := ResponsesToClaudeMessages(sess, []interface{}{
		map[string]interface{}{
			"type":   "function_call_output",
			"output": "Sunny",
		},
	}, "")
	if err == nil {
		t.Fatal("expected error for missing call_id")
	}
	if err.Error() != "转换新消息失败: function_call_output 缺少 call_id" {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestResponsesToClaudeMessages_ToolResultMissingToolUseID(t *testing.T) {
	sess := &session.Session{
		Messages: []types.ResponsesItem{},
	}

	// tool_result 但缺少 tool_use_id
	newInput := []interface{}{
		map[string]interface{}{
			"type": "tool_result",
			"content": map[string]interface{}{
				"content": "some result",
			},
		},
	}

	_, _, err := ResponsesToClaudeMessages(sess, newInput, "")
	if err == nil {
		t.Error("期望返回错误，但成功了")
	}
	if err.Error() != "转换新消息失败: tool_result 缺少 tool_use_id" {
		t.Errorf("错误信息不匹配: %v", err)
	}
}

func TestResponsesToClaudeMessages_FunctionCallDefaultsCallIDToName(t *testing.T) {
	sess := &session.Session{Messages: []types.ResponsesItem{}}

	messages, _, err := ResponsesToClaudeMessages(sess, []interface{}{
		map[string]interface{}{
			"type":      "function_call",
			"name":      "get_weather",
			"arguments": `{"location":"Tokyo"}`,
		},
	}, "")
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}
	if len(messages) != 1 {
		t.Fatalf("期望 1 条消息，实际 %d", len(messages))
	}
	content := messages[0].Content.([]types.ClaudeContent)
	if content[0].ID != "get_weather" {
		t.Fatalf("默认 call_id 应回退到 name，实际 %#v", content[0])
	}
}
