package converters

import (
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// ============== extractTextFromContent 测试 ==============

func TestExtractTextFromContent_String(t *testing.T) {
	content := "Hello, world!"
	result := extractTextFromContent(content)

	if result != "Hello, world!" {
		t.Errorf("期望 'Hello, world!'，实际得到 '%s'", result)
	}
}

func TestExtractTextFromContent_ContentBlockArray(t *testing.T) {
	content := []interface{}{
		map[string]interface{}{
			"type": "input_text",
			"text": "First message",
		},
		map[string]interface{}{
			"type": "input_text",
			"text": "Second message",
		},
	}

	result := extractTextFromContent(content)
	expected := "First message\nSecond message"

	if result != expected {
		t.Errorf("期望 '%s'，实际得到 '%s'", expected, result)
	}
}

func TestExtractTextFromContent_MixedTypes(t *testing.T) {
	content := []interface{}{
		map[string]interface{}{
			"type": "input_text",
			"text": "User message",
		},
		map[string]interface{}{
			"type": "output_text",
			"text": "Assistant message",
		},
		map[string]interface{}{
			"type": "unknown",
			"text": "Should be ignored",
		},
	}

	result := extractTextFromContent(content)
	expected := "User message\nAssistant message"

	if result != expected {
		t.Errorf("期望 '%s'，实际得到 '%s'", expected, result)
	}
}

func TestExtractTextFromContent_EmptyArray(t *testing.T) {
	content := []interface{}{}
	result := extractTextFromContent(content)

	if result != "" {
		t.Errorf("期望空字符串，实际得到 '%s'", result)
	}
}

// ============== OpenAI 转换器测试 ==============

func TestOpenAIChatConverter_WithInstructions(t *testing.T) {
	converter := &OpenAIChatConverter{}
	sess := &session.Session{
		ID:       "sess_test",
		Messages: []types.ResponsesItem{},
	}

	req := &types.ResponsesRequest{
		Model:        "gpt-4",
		Instructions: "You are a helpful assistant.",
		Input:        "Hello!",
		MaxTokens:    100,
		Temperature:  0.7,
	}

	result, err := converter.ToProviderRequest(sess, req)
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	resultMap, ok := result.(map[string]interface{})
	if !ok {
		t.Fatal("结果不是 map[string]interface{}")
	}

	// 检查 model
	if resultMap["model"] != "gpt-4" {
		t.Errorf("期望 model 为 'gpt-4'，实际为 '%v'", resultMap["model"])
	}

	// 检查 messages
	messages, ok := resultMap["messages"].([]map[string]interface{})
	if !ok {
		t.Fatal("messages 不是正确的类型")
	}

	if len(messages) != 2 {
		t.Fatalf("期望 2 条消息（system + user），实际为 %d", len(messages))
	}

	// 检查第一条是 system
	if messages[0]["role"] != "system" {
		t.Errorf("第一条消息应该是 system，实际为 '%v'", messages[0]["role"])
	}
	if messages[0]["content"] != "You are a helpful assistant." {
		t.Errorf("system 内容不匹配")
	}

	// 检查第二条是 user
	if messages[1]["role"] != "user" {
		t.Errorf("第二条消息应该是 user，实际为 '%v'", messages[1]["role"])
	}
	if messages[1]["content"] != "Hello!" {
		t.Errorf("user 内容不匹配")
	}

	// 检查其他参数
	if resultMap["max_tokens"] != 100 {
		t.Errorf("max_tokens 不匹配")
	}
	if resultMap["temperature"] != 0.7 {
		t.Errorf("temperature 不匹配")
	}
}

func TestOpenAIChatConverter_WithMessageType(t *testing.T) {
	converter := &OpenAIChatConverter{}
	sess := &session.Session{
		ID:       "sess_test",
		Messages: []types.ResponsesItem{},
	}

	req := &types.ResponsesRequest{
		Model: "gpt-4",
		Input: []interface{}{
			map[string]interface{}{
				"type": "message",
				"role": "user",
				"content": []interface{}{
					map[string]interface{}{
						"type": "input_text",
						"text": "Hello from message type!",
					},
				},
			},
		},
	}

	result, err := converter.ToProviderRequest(sess, req)
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	resultMap := result.(map[string]interface{})
	messages := resultMap["messages"].([]map[string]interface{})

	if len(messages) != 1 {
		t.Fatalf("期望 1 条消息，实际为 %d", len(messages))
	}

	if messages[0]["role"] != "user" {
		t.Errorf("角色应该是 user")
	}
	assertOpenAIChatTextContent(t, "Hello from message type!", messages[0]["content"])
}

func TestOpenAIChatConverter_WithImageMessageContent(t *testing.T) {
	converter := &OpenAIChatConverter{}
	sess := &session.Session{
		ID:       "sess_test",
		Messages: []types.ResponsesItem{},
	}

	req := &types.ResponsesRequest{
		Model: "mimo-v2.5-pro",
		Input: []interface{}{
			map[string]interface{}{
				"type": "message",
				"role": "user",
				"content": []interface{}{
					map[string]interface{}{
						"type": "input_text",
						"text": "描述这张图片",
					},
					map[string]interface{}{
						"type":      "input_image",
						"image_url": "data:image/png;base64,abc",
						"detail":    "high",
					},
				},
			},
		},
	}

	result, err := converter.ToProviderRequest(sess, req)
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	resultMap := result.(map[string]interface{})
	messages := resultMap["messages"].([]map[string]interface{})
	content, ok := messages[0]["content"].([]map[string]interface{})
	if !ok {
		t.Fatalf("content 应为多模态数组，实际为 %#v", messages[0]["content"])
	}
	if len(content) != 2 {
		t.Fatalf("期望 2 个 content block，实际为 %d", len(content))
	}
	if content[0]["type"] != "text" || content[0]["text"] != "描述这张图片" {
		t.Fatalf("文本 block 不匹配: %#v", content[0])
	}
	imageURL, ok := content[1]["image_url"].(map[string]interface{})
	if !ok || content[1]["type"] != "image_url" {
		t.Fatalf("图片 block 不匹配: %#v", content[1])
	}
	if imageURL["url"] != "data:image/png;base64,abc" || imageURL["detail"] != "high" {
		t.Fatalf("图片 URL 不匹配: %#v", imageURL)
	}
}

// ============== Claude 转换器测试 ==============

func TestClaudeConverter_WithInstructions(t *testing.T) {
	converter := &ClaudeConverter{}
	sess := &session.Session{
		ID:       "sess_test",
		Messages: []types.ResponsesItem{},
	}

	req := &types.ResponsesRequest{
		Model:        "claude-3-opus",
		Instructions: "You are Claude.",
		Input:        "Hello!",
		MaxTokens:    1000,
	}

	result, err := converter.ToProviderRequest(sess, req)
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	resultMap := result.(map[string]interface{})

	// 检查 system 参数（Claude 使用独立的 system 字段）
	if resultMap["system"] != "You are Claude." {
		t.Errorf("system 参数不匹配")
	}

	// 检查 messages
	messages, ok := resultMap["messages"].([]types.ClaudeMessage)
	if !ok {
		t.Fatal("messages 不是正确的类型")
	}

	if len(messages) != 1 {
		t.Fatalf("期望 1 条消息，实际为 %d", len(messages))
	}

	if messages[0].Role != "user" {
		t.Errorf("角色应该是 user")
	}
}

// ============== 工厂模式测试 ==============

func TestConverterFactory(t *testing.T) {
	tests := []struct {
		serviceType  string
		expectedType string
	}{
		{"openai", "*converters.OpenAIChatConverter"},
		{"claude", "*converters.ClaudeConverter"},
		{"responses", "*converters.ResponsesPassthroughConverter"},
		{"unknown", "*converters.OpenAIChatConverter"}, // 默认
	}

	for _, tt := range tests {
		t.Run(tt.serviceType, func(t *testing.T) {
			converter := NewConverter(tt.serviceType)
			if converter == nil {
				t.Errorf("工厂返回 nil")
			}
			// 检查类型（简单验证）
			if converter.GetProviderName() == "" {
				t.Errorf("GetProviderName 返回空字符串")
			}
		})
	}
}

// ============== 会话历史测试 ==============

func TestOpenAIChatConverter_WithSessionHistory(t *testing.T) {
	converter := &OpenAIChatConverter{}
	sess := &session.Session{
		ID: "sess_test",
		Messages: []types.ResponsesItem{
			{
				Type:    "message",
				Role:    "user",
				Content: "Previous user message",
			},
			{
				Type:    "message",
				Role:    "assistant",
				Content: "Previous assistant message",
			},
		},
	}

	req := &types.ResponsesRequest{
		Model: "gpt-4",
		Input: "New user message",
	}

	result, err := converter.ToProviderRequest(sess, req)
	if err != nil {
		t.Fatalf("转换失败: %v", err)
	}

	resultMap := result.(map[string]interface{})
	messages := resultMap["messages"].([]map[string]interface{})

	// 应该有 3 条消息：2 条历史 + 1 条新消息
	if len(messages) != 3 {
		t.Fatalf("期望 3 条消息，实际为 %d", len(messages))
	}

	// 检查顺序
	if messages[0]["content"] != "Previous user message" {
		t.Errorf("第一条消息内容不匹配")
	}
	if messages[1]["content"] != "Previous assistant message" {
		t.Errorf("第二条消息内容不匹配")
	}
	if messages[2]["content"] != "New user message" {
		t.Errorf("第三条消息内容不匹配")
	}
}

// ============== FinishReason 映射测试 ==============

func TestOpenAIFinishReasonToAnthropic(t *testing.T) {
	tests := []struct {
		input    string
		expected string
	}{
		{"stop", "end_turn"},
		{"length", "max_tokens"},
		{"tool_calls", "tool_use"},
		{"function_call", "tool_use"},
		{"content_filter", "refusal"},
		{"empty", "end_turn"},
		{"unknown_reason", "unknown_reason"}, // 未知原因透传
	}

	for _, tt := range tests {
		t.Run(tt.input, func(t *testing.T) {
			result := OpenAIFinishReasonToAnthropic(tt.input)
			if result != tt.expected {
				t.Errorf("OpenAIFinishReasonToAnthropic(%q) = %q, want %q", tt.input, result, tt.expected)
			}
		})
	}
}

func TestAnthropicStopReasonToOpenAI(t *testing.T) {
	tests := []struct {
		input    string
		expected string
	}{
		{"end_turn", "stop"},
		{"max_tokens", "length"},
		{"stop_sequence", "stop"},
		{"pause_turn", "stop"},
		{"tool_use", "tool_calls"},
		{"refusal", "content_filter"},
		{"empty", "stop"},
		{"unknown_reason", "unknown_reason"}, // 未知原因透传
	}

	for _, tt := range tests {
		t.Run(tt.input, func(t *testing.T) {
			result := AnthropicStopReasonToOpenAI(tt.input)
			if result != tt.expected {
				t.Errorf("AnthropicStopReasonToOpenAI(%q) = %q, want %q", tt.input, result, tt.expected)
			}
		})
	}
}

func TestOpenAIFinishReasonToResponses(t *testing.T) {
	tests := []struct {
		input    string
		expected string
	}{
		{"stop", "completed"},
		{"tool_calls", "completed"},
		{"function_call", "completed"},
		{"length", "incomplete"},
		{"content_filter", "failed"},
		{"empty", "completed"},
		{"unknown_reason", "incomplete"}, // 未知原因视为未完成
	}

	for _, tt := range tests {
		t.Run(tt.input, func(t *testing.T) {
			result := OpenAIFinishReasonToResponses(tt.input)
			if result != tt.expected {
				t.Errorf("OpenAIFinishReasonToResponses(%q) = %q, want %q", tt.input, result, tt.expected)
			}
		})
	}
}
