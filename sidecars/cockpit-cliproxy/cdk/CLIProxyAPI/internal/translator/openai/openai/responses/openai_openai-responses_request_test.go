package responses

import (
	"bytes"
	"encoding/json"
	"testing"

	"github.com/tidwall/gjson"
)

func prettyJSONForTest(raw []byte) string {
	if !gjson.ValidBytes(raw) {
		return string(raw)
	}
	var out bytes.Buffer
	if err := json.Indent(&out, raw, "", "  "); err != nil {
		return string(raw)
	}
	return out.String()
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_MergeConsecutiveFunctionCalls(t *testing.T) {
	raw := []byte(`{
		"input": [
			{"type":"function_call","call_id":"exec_command:0","name":"exec_command","arguments":"{\"cmd\":\"ls\"}"},
			{"type":"function_call","call_id":"exec_command:1","name":"exec_command","arguments":"{\"cmd\":\"pwd\"}"},
			{"type":"function_call_output","call_id":"exec_command:0","output":"ok0"},
			{"type":"function_call_output","call_id":"exec_command:1","output":"ok1"}
		]
	}`)
	t.Logf("input json:\n%s", prettyJSONForTest(raw))

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("kimi-k2.6", raw, true)
	t.Logf("output json:\n%s", prettyJSONForTest(out))

	msgs := gjson.GetBytes(out, "messages")
	if !msgs.Exists() || !msgs.IsArray() {
		t.Fatalf("messages should be an array")
	}
	if got := len(msgs.Array()); got != 3 {
		t.Fatalf("messages count = %d, want %d", got, 3)
	}

	if got := gjson.GetBytes(out, "messages.0.role").String(); got != "assistant" {
		t.Fatalf("messages.0.role = %q, want %q", got, "assistant")
	}
	if got := len(gjson.GetBytes(out, "messages.0.tool_calls").Array()); got != 2 {
		t.Fatalf("messages.0.tool_calls length = %d, want %d", got, 2)
	}
	if got := gjson.GetBytes(out, "messages.0.tool_calls.0.id").String(); got != "exec_command:0" {
		t.Fatalf("messages.0.tool_calls.0.id = %q, want %q", got, "exec_command:0")
	}
	if got := gjson.GetBytes(out, "messages.0.tool_calls.1.id").String(); got != "exec_command:1" {
		t.Fatalf("messages.0.tool_calls.1.id = %q, want %q", got, "exec_command:1")
	}

	if got := gjson.GetBytes(out, "messages.1.tool_call_id").String(); got != "exec_command:0" {
		t.Fatalf("messages.1.tool_call_id = %q, want %q", got, "exec_command:0")
	}
	if got := gjson.GetBytes(out, "messages.2.tool_call_id").String(); got != "exec_command:1" {
		t.Fatalf("messages.2.tool_call_id = %q, want %q", got, "exec_command:1")
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_SplitFunctionCallsWhenInterrupted(t *testing.T) {
	raw := []byte(`{
		"input": [
			{"type":"function_call","call_id":"call_a","name":"tool_a","arguments":"{}"},
			{"type":"message","role":"user","content":"next"},
			{"type":"function_call","call_id":"call_b","name":"tool_b","arguments":"{}"}
		]
	}`)
	t.Logf("input json:\n%s", prettyJSONForTest(raw))

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("kimi-k2.6", raw, false)
	t.Logf("output json:\n%s", prettyJSONForTest(out))

	if got := len(gjson.GetBytes(out, "messages").Array()); got != 3 {
		t.Fatalf("messages count = %d, want %d", got, 3)
	}
	if got := gjson.GetBytes(out, "messages.0.tool_calls.0.id").String(); got != "call_a" {
		t.Fatalf("messages.0.tool_calls.0.id = %q, want %q", got, "call_a")
	}
	if got := gjson.GetBytes(out, "messages.2.tool_calls.0.id").String(); got != "call_b" {
		t.Fatalf("messages.2.tool_calls.0.id = %q, want %q", got, "call_b")
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_DefersMessageUntilToolOutput(t *testing.T) {
	raw := []byte(`{
		"input": [
			{"type":"function_call","call_id":"call_x","name":"exec_command","arguments":"{\"cmd\":\"echo hi\"}"},
			{"type":"message","role":"user","content":"Approved command prefix saved"},
			{"type":"function_call_output","call_id":"call_x","output":"ok"},
			{"type":"message","role":"user","content":"next"}
		]
	}`)
	t.Logf("input json:\n%s", prettyJSONForTest(raw))

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("kimi-k2.6", raw, true)
	t.Logf("output json:\n%s", prettyJSONForTest(out))

	if got := len(gjson.GetBytes(out, "messages").Array()); got != 4 {
		t.Fatalf("messages count = %d, want %d", got, 4)
	}
	if got := gjson.GetBytes(out, "messages.0.role").String(); got != "assistant" {
		t.Fatalf("messages.0.role = %q, want %q", got, "assistant")
	}
	if got := gjson.GetBytes(out, "messages.1.role").String(); got != "tool" {
		t.Fatalf("messages.1.role = %q, want %q", got, "tool")
	}
	if got := gjson.GetBytes(out, "messages.1.tool_call_id").String(); got != "call_x" {
		t.Fatalf("messages.1.tool_call_id = %q, want %q", got, "call_x")
	}
	if got := gjson.GetBytes(out, "messages.2.role").String(); got != "user" {
		t.Fatalf("messages.2.role = %q, want %q", got, "user")
	}
	if got := gjson.GetBytes(out, "messages.2.content").String(); got != "Approved command prefix saved" {
		t.Fatalf("messages.2.content = %q, want %q", got, "Approved command prefix saved")
	}
	if got := gjson.GetBytes(out, "messages.3.content").String(); got != "next" {
		t.Fatalf("messages.3.content = %q, want %q", got, "next")
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_CustomToolUsesInputWrapper(t *testing.T) {
	raw := []byte(`{
		"tools": [
			{"type":"custom","name":"exec","description":"Run shell input"}
		],
		"input": [
			{"type":"custom_tool_call","call_id":"call_custom","name":"exec","input":"ls -la"},
			{"type":"custom_tool_call_output","call_id":"call_custom","output":"ok"}
		]
	}`)

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("kimi-k2.6", raw, true)

	if got := gjson.GetBytes(out, "tools.0.type").String(); got != "function" {
		t.Fatalf("tools.0.type = %q, want function", got)
	}
	if got := gjson.GetBytes(out, "tools.0.function.name").String(); got != "exec" {
		t.Fatalf("tools.0.function.name = %q, want exec", got)
	}
	if got := gjson.GetBytes(out, "tools.0.function.parameters.properties.input.type").String(); got != "string" {
		t.Fatalf("custom tool input parameter type = %q, want string", got)
	}
	args := gjson.GetBytes(out, "messages.0.tool_calls.0.function.arguments").String()
	if got := gjson.Get(args, "input").String(); got != "ls -la" {
		t.Fatalf("custom tool arguments input = %q, want ls -la; args=%s", got, args)
	}
	if got := gjson.GetBytes(out, "messages.1.tool_call_id").String(); got != "call_custom" {
		t.Fatalf("tool output id = %q, want call_custom", got)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_IncludesUsageForStreaming(t *testing.T) {
	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("kimi-k2.6", []byte(`{"input":"hello"}`), true)

	if got := gjson.GetBytes(out, "stream_options.include_usage").Bool(); !got {
		t.Fatalf("stream_options.include_usage = %v, want true; out=%s", got, out)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_ConvertsInputImages(t *testing.T) {
	raw := []byte(`{
		"input": [{
			"type": "message",
			"role": "user",
			"content": [
				{"type":"input_text","text":"describe"},
				{"type":"input_image","image_url":"data:image/png;base64,AAAA"},
				{"type":"input_image","image_url":{"url":"https://example.com/image.png","detail":"high"}}
			]
		}]
	}`)

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("qwen-vl-plus", raw, false)

	if got := gjson.GetBytes(out, "messages.0.content.1.type").String(); got != "image_url" {
		t.Fatalf("content.1.type = %q, want image_url; out=%s", got, out)
	}
	if got := gjson.GetBytes(out, "messages.0.content.1.image_url.url").String(); got != "data:image/png;base64,AAAA" {
		t.Fatalf("content.1.image_url.url = %q; out=%s", got, out)
	}
	if got := gjson.GetBytes(out, "messages.0.content.2.image_url.url").String(); got != "https://example.com/image.png" {
		t.Fatalf("content.2.image_url.url = %q; out=%s", got, out)
	}
	if got := gjson.GetBytes(out, "messages.0.content.2.image_url.detail").String(); got != "high" {
		t.Fatalf("content.2.image_url.detail = %q; out=%s", got, out)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_CanonicalizesEmptyArguments(t *testing.T) {
	raw := []byte(`{
		"input": [
			{"type":"function_call","call_id":"call_empty","name":"noop","arguments":""}
		]
	}`)

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("kimi-k2.6", raw, true)

	if got := gjson.GetBytes(out, "messages.0.tool_calls.0.function.arguments").String(); got != "{}" {
		t.Fatalf("arguments = %q, want {}", got)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_CollapsesSystemMessagesToHead(t *testing.T) {
	raw := []byte(`{
		"instructions": "root system",
		"input": [
			{"type":"message","role":"user","content":"first"},
			{"type":"message","role":"developer","content":"developer note"},
			{"type":"message","role":"user","content":"second"}
		]
	}`)

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("kimi-k2.6", raw, true)

	if got := gjson.GetBytes(out, "messages.0.role").String(); got != "system" {
		t.Fatalf("messages.0.role = %q, want system; out=%s", got, out)
	}
	if got := gjson.GetBytes(out, "messages.0.content").String(); got != "root system\n\ndeveloper note" {
		t.Fatalf("messages.0.content = %q", got)
	}
	for i, msg := range gjson.GetBytes(out, "messages").Array()[1:] {
		if got := msg.Get("role").String(); got == "system" {
			t.Fatalf("unexpected system role after head at index %d; out=%s", i+1, out)
		}
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_ExposesToolSearchAndNamespaceTools(t *testing.T) {
	raw := []byte(`{
		"tools": [{"type":"tool_search"}],
		"input": [
			{"type":"tool_search_call","call_id":"call_search","arguments":{"query":"gmail search","limit":5}},
			{"type":"tool_search_output","call_id":"call_search","output":"loaded","tools":[{
				"type":"namespace",
				"name":"mcp__codex_apps__gmail",
				"tools":[{"type":"function","name":"_search_emails","description":"Search email","parameters":{"type":"object"}}]
			}]}
		]
	}`)

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("kimi-k2.6", raw, true)

	names := map[string]bool{}
	for _, tool := range gjson.GetBytes(out, "tools").Array() {
		names[tool.Get("function.name").String()] = true
	}
	if !names["tool_search"] {
		t.Fatalf("tool_search was not exposed; out=%s", out)
	}
	if !names["mcp__codex_apps__gmail___search_emails"] {
		t.Fatalf("namespace tool was not exposed; out=%s", out)
	}
	if got := gjson.GetBytes(out, "messages.0.tool_calls.0.function.name").String(); got != "tool_search" {
		t.Fatalf("tool call name = %q, want tool_search", got)
	}
}
