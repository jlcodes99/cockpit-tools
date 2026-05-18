package converters

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/tidwall/gjson"
)

func extractResponseCompletedUsage(t *testing.T, events []string) map[string]interface{} {
	t.Helper()
	for _, event := range events {
		if !strings.Contains(event, "event: response.completed") {
			continue
		}
		for _, line := range strings.Split(event, "\n") {
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			jsonStr := strings.TrimPrefix(line, "data: ")
			var payload map[string]interface{}
			if err := json.Unmarshal([]byte(jsonStr), &payload); err != nil {
				continue
			}
			response, ok := payload["response"].(map[string]interface{})
			if !ok {
				continue
			}
			usage, ok := response["usage"].(map[string]interface{})
			if ok {
				return usage
			}
		}
	}
	t.Fatalf("未找到 response.completed usage 事件: %v", events)
	return nil
}

func TestConvertResponsesToOpenAIChatRequest(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		model    string
		stream   bool
		validate func(t *testing.T, result []byte)
	}{
		{
			name: "基本文本输入",
			input: `{
				"model": "gpt-4",
				"input": "Hello, world!",
				"instructions": "You are a helpful assistant."
			}`,
			model:  "gpt-4o",
			stream: false,
			validate: func(t *testing.T, result []byte) {
				root := gjson.ParseBytes(result)
				if root.Get("model").String() != "gpt-4o" {
					t.Errorf("model should be gpt-4o, got %s", root.Get("model").String())
				}
				if root.Get("stream").Bool() != false {
					t.Error("stream should be false")
				}
				messages := root.Get("messages").Array()
				if len(messages) != 2 {
					t.Errorf("should have 2 messages (system + user), got %d", len(messages))
				}
				if messages[0].Get("role").String() != "system" {
					t.Error("first message should be system")
				}
				if messages[1].Get("role").String() != "user" {
					t.Error("second message should be user")
				}
			},
		},
		{
			name: "带 tools 的请求",
			input: `{
				"model": "gpt-4",
				"input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What's the weather?"}]}],
				"tools": [
					{
						"name": "get_weather",
						"description": "Get weather info",
						"parameters": {"type": "object", "properties": {"location": {"type": "string"}}}
					}
				]
			}`,
			model:  "gpt-4o",
			stream: true,
			validate: func(t *testing.T, result []byte) {
				root := gjson.ParseBytes(result)
				if root.Get("stream").Bool() != true {
					t.Error("stream should be true")
				}
				tools := root.Get("tools").Array()
				if len(tools) != 1 {
					t.Errorf("should have 1 tool, got %d", len(tools))
				}
				if tools[0].Get("function.name").String() != "get_weather" {
					t.Error("tool name should be get_weather")
				}
			},
		},
		{
			name: "function_call 和 function_call_output",
			input: `{
				"model": "gpt-4",
				"input": [
					{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What's the weather in NYC?"}]},
					{"type": "function_call", "call_id": "call_123", "name": "get_weather", "arguments": "{\"location\": \"NYC\"}"},
					{"type": "function_call_output", "call_id": "call_123", "output": "Sunny, 72°F"}
				]
			}`,
			model:  "gpt-4o",
			stream: false,
			validate: func(t *testing.T, result []byte) {
				root := gjson.ParseBytes(result)
				messages := root.Get("messages").Array()
				if len(messages) != 3 {
					t.Errorf("should have 3 messages, got %d", len(messages))
				}
				// 第二条消息应该是 assistant with tool_calls
				if messages[1].Get("role").String() != "assistant" {
					t.Error("second message should be assistant")
				}
				if !messages[1].Get("tool_calls").Exists() {
					t.Error("assistant message should have tool_calls")
				}
				// 第三条消息应该是 tool
				if messages[2].Get("role").String() != "tool" {
					t.Error("third message should be tool")
				}
			},
		},
		{
			name: "多模态图片输入保留为 Chat content array",
			input: `{
				"model": "mimo-v2.5-pro",
				"input": [{"type": "message", "role": "user", "content": [
					{"type": "input_text", "text": "描述这张图片"},
					{"type": "input_image", "image_url": "data:image/png;base64,abc", "detail": "high"}
				]}]
			}`,
			model:  "mimo-v2.5-pro",
			stream: false,
			validate: func(t *testing.T, result []byte) {
				root := gjson.ParseBytes(result)
				content := root.Get("messages.0.content")
				if !content.IsArray() {
					t.Fatalf("content should be array, got %s", content.Raw)
				}
				if content.Get("0.type").String() != "text" || content.Get("0.text").String() != "描述这张图片" {
					t.Fatalf("text block mismatch: %s", content.Get("0").Raw)
				}
				if content.Get("1.type").String() != "image_url" {
					t.Fatalf("image block type mismatch: %s", content.Get("1").Raw)
				}
				if content.Get("1.image_url.url").String() != "data:image/png;base64,abc" {
					t.Fatalf("image url mismatch: %s", content.Get("1").Raw)
				}
				if content.Get("1.image_url.detail").String() != "high" {
					t.Fatalf("image detail mismatch: %s", content.Get("1").Raw)
				}
			},
		},
		{
			name: "tool_choice object 保真",
			input: `{
				"model": "gpt-4",
				"input": "Call a tool",
				"tool_choice": {"type": "function", "function": {"name": "get_weather"}}
			}`,
			model:  "gpt-4o",
			stream: false,
			validate: func(t *testing.T, result []byte) {
				root := gjson.ParseBytes(result)
				if root.Get("tool_choice.type").String() != "function" {
					t.Fatalf("tool_choice.type should be function, got %s", root.Get("tool_choice.type").String())
				}
				if root.Get("tool_choice.function.name").String() != "get_weather" {
					t.Fatalf("tool_choice.function.name should be get_weather, got %s", root.Get("tool_choice.function.name").String())
				}
			},
		},

		{
			name: "tools 缺失 required 字段时自动补齐 []",
			input: `{
				"model": "gpt-5-codex",
				"input": "list mcp resources",
				"tools": [
					{
						"type": "function",
						"name": "list_mcp_resources",
						"description": "Lists resources provided by MCP servers.",
						"strict": false,
						"parameters": {
							"type": "object",
							"properties": {
								"cursor": {"type": "string"},
								"server": {"type": "string"}
							},
							"additionalProperties": false
						}
					}
				]
			}`,
			model:  "gpt-5-codex",
			stream: false,
			validate: func(t *testing.T, result []byte) {
				root := gjson.ParseBytes(result)
				tools := root.Get("tools").Array()
				if len(tools) != 1 {
					t.Fatalf("should have 1 tool, got %d", len(tools))
				}
				tool := tools[0]
				if tool.Get("function.name").String() != "list_mcp_resources" {
					t.Fatalf("tool name mismatch: %s", tool.Raw)
				}
				params := tool.Get("function.parameters")
				if params.Get("type").String() != "object" {
					t.Fatalf("parameters.type should be object: %s", params.Raw)
				}
				required := params.Get("required")
				if !required.Exists() || !required.IsArray() {
					t.Fatalf("parameters.required should exist and be array, got %s", params.Raw)
				}
				if params.Get("additionalProperties").Bool() != false {
					t.Fatalf("additionalProperties should be preserved: %s", params.Raw)
				}
			},
		},
		{
			name: "非 function 类型的工具应被跳过",
			input: `{
				"model": "gpt-5-codex",
				"input": "search the web",
				"tools": [
					{"type": "web_search"},
					{"type": "custom", "name": "grep"},
					{"type": "function", "name": "do_thing", "parameters": {"type": "object", "properties": {}}}
				]
			}`,
			model:  "gpt-5-codex",
			stream: false,
			validate: func(t *testing.T, result []byte) {
				root := gjson.ParseBytes(result)
				tools := root.Get("tools").Array()
				if len(tools) != 1 {
					t.Fatalf("expected 1 tool after filtering, got %d (%s)", len(tools), root.Get("tools").Raw)
				}
				if tools[0].Get("function.name").String() != "do_thing" {
					t.Fatalf("should keep only function tool, got %s", tools[0].Raw)
				}
			},
		},
		{
			name: "reasoning effort 转换",
			input: `{
				"model": "o1-mini",
				"input": "Think about this",
				"reasoning": {"effort": "high"}
			}`,
			model:  "o1-mini",
			stream: false,
			validate: func(t *testing.T, result []byte) {
				root := gjson.ParseBytes(result)
				if root.Get("reasoning_effort").String() != "high" {
					t.Errorf("reasoning_effort should be high, got %s", root.Get("reasoning_effort").String())
				}
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := ConvertResponsesToOpenAIChatRequest(tt.model, []byte(tt.input), tt.stream)
			tt.validate(t, result)
		})
	}
}

func TestConvertOpenAIChatToResponses_Stream(t *testing.T) {
	ctx := context.Background()

	// 模拟 OpenAI Chat Completions SSE 流
	sseLines := []string{
		`data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":" world!"},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}`,
		`data: [DONE]`,
	}

	originalReq := []byte(`{"model":"gpt-4o","input":"Hi"}`)

	var state any
	var allEvents []string

	for _, line := range sseLines {
		events := ConvertOpenAIChatToResponses(ctx, "gpt-4o", originalReq, nil, []byte(line), &state)
		allEvents = append(allEvents, events...)
	}

	// 验证事件序列
	if len(allEvents) == 0 {
		t.Fatal("should produce events")
	}

	// 检查是否有 response.created 事件
	hasCreated := false
	hasInProgress := false
	hasCompleted := false
	hasTextDelta := false

	for _, ev := range allEvents {
		if strings.Contains(ev, "response.created") {
			hasCreated = true
		}
		if strings.Contains(ev, "response.in_progress") {
			hasInProgress = true
		}
		if strings.Contains(ev, "response.completed") {
			hasCompleted = true
		}
		if strings.Contains(ev, "response.output_text.delta") {
			hasTextDelta = true
		}
	}

	if !hasCreated {
		t.Error("should have response.created event")
	}
	if !hasInProgress {
		t.Error("should have response.in_progress event")
	}
	if !hasCompleted {
		t.Error("should have response.completed event")
	}
	if !hasTextDelta {
		t.Error("should have response.output_text.delta event")
	}
}

func TestConvertOpenAIChatToResponses_StreamReasoningContent(t *testing.T) {
	ctx := context.Background()
	sseLines := []string{
		`data: {"id":"chatcmpl-ds","object":"chat.completion.chunk","created":1234567890,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{"reasoning_content":"chat reasoning"},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-ds","object":"chat.completion.chunk","created":1234567890,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{"content":"chat text"},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-ds","object":"chat.completion.chunk","created":1234567890,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}`,
		`data: [DONE]`,
	}

	var state any
	var allEvents []string
	for _, line := range sseLines {
		events := ConvertOpenAIChatToResponses(ctx, "deepseek-v4-pro", []byte(`{"model":"deepseek-v4-pro","input":"hello"}`), nil, []byte(line), &state)
		allEvents = append(allEvents, events...)
	}

	joined := strings.Join(allEvents, "\n")
	if !strings.Contains(joined, `"type":"reasoning"`) {
		t.Fatalf("expected reasoning item, got %v", allEvents)
	}
	if !strings.Contains(joined, `"text":"chat reasoning"`) {
		t.Fatalf("expected reasoning summary text, got %v", allEvents)
	}
	if !strings.Contains(joined, `"delta":"chat text"`) {
		t.Fatalf("expected text delta after reasoning, got %v", allEvents)
	}
}

func TestConvertOpenAIChatToResponses_ToolCall(t *testing.T) {
	ctx := context.Background()

	// 模拟带 tool_call 的 SSE 流
	sseLines := []string{
		`data: {"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"loc"}}]},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ation\": \"NYC\"}"}}]},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}`,
		`data: [DONE]`,
	}

	originalReq := []byte(`{"model":"gpt-4o","input":"What's the weather?","tools":[{"name":"get_weather"}]}`)

	var state any
	var allEvents []string

	for _, line := range sseLines {
		events := ConvertOpenAIChatToResponses(ctx, "gpt-4o", originalReq, nil, []byte(line), &state)
		allEvents = append(allEvents, events...)
	}

	// 验证是否有 function_call 相关事件
	hasFuncAdded := false
	hasFuncDelta := false
	hasFuncDone := false

	for _, ev := range allEvents {
		if strings.Contains(ev, "response.output_item.added") && strings.Contains(ev, "function_call") {
			hasFuncAdded = true
		}
		if strings.Contains(ev, "response.function_call_arguments.delta") {
			hasFuncDelta = true
		}
		if strings.Contains(ev, "response.function_call_arguments.done") {
			hasFuncDone = true
		}
	}

	if !hasFuncAdded {
		t.Error("should have function_call output_item.added event")
	}
	if !hasFuncDelta {
		t.Error("should have function_call_arguments.delta event")
	}
	if !hasFuncDone {
		t.Error("should have function_call_arguments.done event")
	}
}

func TestConvertOpenAIChatToResponses_CustomToolCall(t *testing.T) {
	ctx := context.Background()
	sseLines := []string{
		`data: {"id":"chatcmpl-custom","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_patch","type":"function","function":{"name":"apply_patch_add_file","arguments":""}}]},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-custom","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"docs/test.md\",\"content\":\"# Test\\n\"}"}}]},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-custom","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}`,
		`data: [DONE]`,
	}
	originalReq := []byte(`{"model":"gpt-4o","input":"edit","tools":[{"type":"custom","name":"apply_patch"}],"transformer_metadata":{"codex_tool_compat_enabled":true}}`)

	var state any
	var allEvents []string
	for _, line := range sseLines {
		allEvents = append(allEvents, ConvertOpenAIChatToResponses(ctx, "gpt-4o", originalReq, nil, []byte(line), &state)...)
	}
	joined := strings.Join(allEvents, "\n")
	if strings.Contains(joined, `"type":"function_call"`) {
		t.Fatalf("custom tool stream leaked function_call events: %s", joined)
	}
	if strings.Contains(joined, "response.function_call_arguments.") {
		t.Fatalf("custom tool stream leaked function argument events: %s", joined)
	}
	if !strings.Contains(joined, `"type":"custom_tool_call"`) {
		t.Fatalf("missing custom_tool_call item: %s", joined)
	}
	if !strings.Contains(joined, "response.custom_tool_call_input.delta") {
		t.Fatalf("missing custom tool input delta: %s", joined)
	}
	if !strings.Contains(joined, `*** Add File: docs/test.md`) || strings.Contains(joined, `\n+\n*** End Patch`) {
		t.Fatalf("unexpected patch input: %s", joined)
	}
}

func TestConvertOpenAIChatToResponsesNonStream(t *testing.T) {
	ctx := context.Background()

	// 模拟 OpenAI Chat Completions 非流式响应
	chatResponse := `{
		"id": "chatcmpl-789",
		"object": "chat.completion",
		"created": 1234567890,
		"model": "gpt-4o",
		"choices": [{
			"index": 0,
			"message": {
				"role": "assistant",
				"content": "Hello! How can I help you today?"
			},
			"finish_reason": "stop"
		}],
		"usage": {
			"prompt_tokens": 10,
			"completion_tokens": 8,
			"total_tokens": 18
		}
	}`

	originalReq := []byte(`{"model":"gpt-4o","input":"Hi","instructions":"Be helpful"}`)

	result := ConvertOpenAIChatToResponsesNonStream(ctx, "gpt-4o", originalReq, nil, []byte(chatResponse), nil)

	// 解析结果
	var resp map[string]interface{}
	if err := json.Unmarshal([]byte(result), &resp); err != nil {
		t.Fatalf("failed to parse result: %v", err)
	}

	// 验证基本字段
	if resp["object"] != "response" {
		t.Errorf("object should be response, got %v", resp["object"])
	}
	if resp["status"] != "completed" {
		t.Errorf("status should be completed, got %v", resp["status"])
	}

	// 验证 output
	output, ok := resp["output"].([]interface{})
	if !ok || len(output) == 0 {
		t.Fatal("output should have items")
	}

	msgItem := output[0].(map[string]interface{})
	if msgItem["type"] != "message" {
		t.Errorf("first output item should be message, got %v", msgItem["type"])
	}

	// 验证 usage
	usage, ok := resp["usage"].(map[string]interface{})
	if !ok {
		t.Fatal("usage should exist")
	}
	if usage["input_tokens"].(float64) != 10 {
		t.Errorf("input_tokens should be 10, got %v", usage["input_tokens"])
	}
	if usage["output_tokens"].(float64) != 8 {
		t.Errorf("output_tokens should be 8, got %v", usage["output_tokens"])
	}
}

func TestConvertOpenAIChatToResponsesNonStream_ToolCalls(t *testing.T) {
	ctx := context.Background()

	// 模拟带 tool_calls 的响应
	chatResponse := `{
		"id": "chatcmpl-tool",
		"object": "chat.completion",
		"created": 1234567890,
		"model": "gpt-4o",
		"choices": [{
			"index": 0,
			"message": {
				"role": "assistant",
				"content": null,
				"tool_calls": [
					{
						"id": "call_xyz",
						"type": "function",
						"function": {
							"name": "search",
							"arguments": "{\"query\": \"test\"}"
						}
					}
				]
			},
			"finish_reason": "tool_calls"
		}],
		"usage": {"prompt_tokens": 5, "completion_tokens": 10, "total_tokens": 15}
	}`

	originalReq := []byte(`{"model":"gpt-4o","input":"Search for test"}`)

	result := ConvertOpenAIChatToResponsesNonStream(ctx, "gpt-4o", originalReq, nil, []byte(chatResponse), nil)

	var resp map[string]interface{}
	if err := json.Unmarshal([]byte(result), &resp); err != nil {
		t.Fatalf("failed to parse result: %v", err)
	}

	output, ok := resp["output"].([]interface{})
	if !ok || len(output) == 0 {
		t.Fatal("output should have items")
	}

	// 查找 function_call item
	var funcItem map[string]interface{}
	for _, item := range output {
		itemMap := item.(map[string]interface{})
		if itemMap["type"] == "function_call" {
			funcItem = itemMap
			break
		}
	}

	if funcItem == nil {
		t.Fatal("should have function_call item")
	}

	if funcItem["name"] != "search" {
		t.Errorf("function name should be search, got %v", funcItem["name"])
	}
	if funcItem["call_id"] != "call_xyz" {
		t.Errorf("call_id should be call_xyz, got %v", funcItem["call_id"])
	}
}

func TestConvertOpenAIChatToResponsesNonStream_CustomToolCall(t *testing.T) {
	ctx := context.Background()
	chatResponse := `{
		"id": "chatcmpl-custom",
		"object": "chat.completion",
		"created": 1234567890,
		"model": "gpt-4o",
		"choices": [{
			"index": 0,
			"message": {
				"role": "assistant",
				"content": null,
				"tool_calls": [{
					"id": "call_patch",
					"type": "function",
					"function": {
						"name": "apply_patch_add_file",
						"arguments": "{\"path\":\"docs/test.md\",\"content\":\"# Test\\n\"}"
					}
				}]
			},
			"finish_reason": "tool_calls"
		}],
		"usage": {"prompt_tokens": 5, "completion_tokens": 10, "total_tokens": 15}
	}`
	originalReq := []byte(`{"model":"gpt-4o","input":"edit","tools":[{"type":"custom","name":"apply_patch"}],"transformer_metadata":{"codex_tool_compat_enabled":true}}`)

	result := ConvertOpenAIChatToResponsesNonStream(ctx, "gpt-4o", originalReq, nil, []byte(chatResponse), nil)

	var resp map[string]interface{}
	if err := json.Unmarshal([]byte(result), &resp); err != nil {
		t.Fatal(err)
	}
	output := resp["output"].([]interface{})
	item := output[0].(map[string]interface{})
	if item["type"] != "custom_tool_call" {
		t.Fatalf("item type = %v, result = %s", item["type"], result)
	}
	if item["name"] != "apply_patch" {
		t.Fatalf("name = %v", item["name"])
	}
	if _, ok := item["output"]; ok {
		t.Fatalf("custom_tool_call should not contain output: %#v", item)
	}
	if got := item["input"].(string); got != "*** Begin Patch\n*** Add File: docs/test.md\n+# Test\n*** End Patch" {
		t.Fatalf("input = %q", got)
	}
}

func TestConvertOpenAIChatToResponses_Stream_ClaudeCacheTotalTokens(t *testing.T) {
	ctx := context.Background()
	sseLines := []string{
		`data: {"id":"msg-claude-cache","object":"chat.completion.chunk","created":1234567890,"model":"claude","choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},"finish_reason":null}]}`,
		`data: {"id":"msg-claude-cache","object":"chat.completion.chunk","created":1234567890,"model":"claude","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":10,"cache_read_input_tokens":30}}`,
		`data: [DONE]`,
	}

	originalReq := []byte(`{"model":"claude","input":"hi"}`)
	var state any
	var allEvents []string
	for _, line := range sseLines {
		allEvents = append(allEvents, ConvertOpenAIChatToResponses(ctx, "claude", originalReq, nil, []byte(line), &state)...)
	}

	usage := extractResponseCompletedUsage(t, allEvents)
	if got := int(usage["total_tokens"].(float64)); got != 160 {
		t.Fatalf("total_tokens = %d, want 160", got)
	}
	if got := int(usage["cache_creation_input_tokens"].(float64)); got != 10 {
		t.Fatalf("cache_creation_input_tokens = %d, want 10", got)
	}
	if got := int(usage["cache_read_input_tokens"].(float64)); got != 30 {
		t.Fatalf("cache_read_input_tokens = %d, want 30", got)
	}
}

func TestConvertOpenAIChatToResponses_Stream_OpenAICacheDetailsNormalizesInput(t *testing.T) {
	ctx := context.Background()
	sseLines := []string{
		`data: {"id":"chatcmpl-openai-cache","object":"chat.completion.chunk","created":1234567890,"model":"gpt-5.5","choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},"finish_reason":null}]}`,
		`data: {"id":"chatcmpl-openai-cache","object":"chat.completion.chunk","created":1234567890,"model":"gpt-5.5","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":38451,"completion_tokens":1275,"total_tokens":39726,"prompt_tokens_details":{"cached_tokens":36608}}}`,
		`data: [DONE]`,
	}

	originalReq := []byte(`{"model":"gpt-5.5","input":"hi"}`)
	var state any
	var allEvents []string
	for _, line := range sseLines {
		allEvents = append(allEvents, ConvertOpenAIChatToResponses(ctx, "gpt-5.5", originalReq, nil, []byte(line), &state)...)
	}

	usage := extractResponseCompletedUsage(t, allEvents)
	if got := int(usage["input_tokens"].(float64)); got != 1843 {
		t.Fatalf("input_tokens = %d, want 1843", got)
	}
	if got := int(usage["total_tokens"].(float64)); got != 39726 {
		t.Fatalf("total_tokens = %d, want 39726", got)
	}
	if _, exists := usage["cache_read_input_tokens"]; exists {
		t.Fatalf("cache_read_input_tokens should not be emitted for OpenAI cache details: %#v", usage)
	}
	details, ok := usage["input_tokens_details"].(map[string]interface{})
	if !ok {
		t.Fatalf("input_tokens_details missing: %#v", usage)
	}
	if got := int(details["cached_tokens"].(float64)); got != 36608 {
		t.Fatalf("cached_tokens = %d, want 36608", got)
	}
}

func TestConvertOpenAIChatToResponsesNonStream_ClaudeCacheTTLTotalFallback(t *testing.T) {
	ctx := context.Background()
	chatResponse := `{
		"id":"chatcmpl-claude-cache",
		"object":"chat.completion",
		"created":1234567890,
		"model":"claude",
		"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
		"usage":{
			"input_tokens":100,
			"output_tokens":20,
			"cache_read_input_tokens":30,
			"cache_creation_5m_input_tokens":7,
			"cache_creation_1h_input_tokens":3
		}
	}`

	result := ConvertOpenAIChatToResponsesNonStream(ctx, "claude", []byte(`{"model":"claude","input":"hi"}`), nil, []byte(chatResponse), nil)
	if got := gjson.Get(result, "usage.total_tokens").Int(); got != 160 {
		t.Fatalf("usage.total_tokens = %d, want 160", got)
	}
}

func TestConvertOpenAIChatToResponsesNonStream_OpenAICacheDetailsNormalizesInput(t *testing.T) {
	ctx := context.Background()
	chatResponse := `{
		"id":"chatcmpl-openai-cache",
		"object":"chat.completion",
		"created":1234567890,
		"model":"gpt-5.5",
		"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
		"usage":{
			"prompt_tokens":38451,
			"completion_tokens":1275,
			"total_tokens":39726,
			"prompt_tokens_details":{"cached_tokens":36608}
		}
	}`

	result := ConvertOpenAIChatToResponsesNonStream(ctx, "gpt-5.5", []byte(`{"model":"gpt-5.5","input":"hi"}`), nil, []byte(chatResponse), nil)
	if got := gjson.Get(result, "usage.input_tokens").Int(); got != 1843 {
		t.Fatalf("usage.input_tokens = %d, want 1843", got)
	}
	if got := gjson.Get(result, "usage.total_tokens").Int(); got != 39726 {
		t.Fatalf("usage.total_tokens = %d, want 39726", got)
	}
	if gjson.Get(result, "usage.cache_read_input_tokens").Exists() {
		t.Fatalf("usage.cache_read_input_tokens should not be emitted: %s", result)
	}
	if got := gjson.Get(result, "usage.input_tokens_details.cached_tokens").Int(); got != 36608 {
		t.Fatalf("usage.input_tokens_details.cached_tokens = %d, want 36608", got)
	}
}
