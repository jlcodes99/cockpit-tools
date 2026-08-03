package responses

import (
	"bytes"
	"encoding/json"
	"strings"
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

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_AdditionalToolsFromResponsesLite(t *testing.T) {
	// Codex Desktop Responses Lite often sends tools=null and ships definitions
	// via additional_tools input items plus custom_tool_call history.
	raw := []byte(`{
		"tools": null,
		"input": [
			{
				"type":"additional_tools",
				"tools":[
					{"type":"custom","name":"apply_patch","description":"Apply a patch"},
					{"type":"function","name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}
				]
			},
			{"type":"custom_tool_call","call_id":"call_patch","name":"apply_patch","input":"*** Begin Patch\n*** End Patch"},
			{"type":"custom_tool_call_output","call_id":"call_patch","output":[{"type":"input_text","text":"applied"}]},
			{"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}]}
		]
	}`)

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("gpt-5.6-sol", raw, true)

	toolNames := map[string]bool{}
	for _, tool := range gjson.GetBytes(out, "tools").Array() {
		toolNames[tool.Get("function.name").String()] = true
	}
	if !toolNames["apply_patch"] || !toolNames["read_file"] {
		t.Fatalf("tools from additional_tools missing; names=%v out=%s", toolNames, out)
	}
	if got := gjson.GetBytes(out, "tools.#(function.name==\"apply_patch\").function.parameters.properties.input.type").String(); got != "string" {
		t.Fatalf("apply_patch parameters = %s", out)
	}
	args := gjson.GetBytes(out, "messages.0.tool_calls.0.function.arguments").String()
	if got := gjson.Get(args, "input").String(); !strings.Contains(got, "Begin Patch") {
		t.Fatalf("custom history args = %q", args)
	}
	if got := gjson.GetBytes(out, "messages.1.content").String(); got != `[{"type":"input_text","text":"applied"}]` {
		t.Fatalf("tool output = %q, want original array JSON", got)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_MovesToolOutputMediaToUserMessage(t *testing.T) {
	raw := []byte(`{
		"input": [
			{"type":"function_call","call_id":"call_a","name":"view_image","arguments":"{}"},
			{"type":"function_call","call_id":"call_b","name":"view_image","arguments":"{}"},
			{"type":"message","role":"user","content":"continue"},
			{"type":"function_call_output","call_id":"call_b","output":{"status":"ok","content":[{"type":"image_url","image_url":{"url":"https://example.com/b.png"}}],"large":9007199254740993}},
			{"type":"function_call_output","call_id":"call_a","output":[{"type":"input_image","image_url":"data:image/png;base64,AQID"}]}
		]
	}`)

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("vision-model", raw, false)
	messages := gjson.GetBytes(out, "messages").Array()
	if len(messages) != 5 {
		t.Fatalf("messages count = %d, want 5; out=%s", len(messages), out)
	}
	if got := messages[1].Get("tool_call_id").String(); got != "call_a" {
		t.Fatalf("first tool reply = %q, want call_a; out=%s", got, out)
	}
	if got := messages[2].Get("tool_call_id").String(); got != "call_b" {
		t.Fatalf("second tool reply = %q, want call_b; out=%s", got, out)
	}
	for _, imageURL := range []string{"data:image/png;base64,AQID", "https://example.com/b.png"} {
		if strings.Contains(messages[1].Get("content").String(), imageURL) || strings.Contains(messages[2].Get("content").String(), imageURL) {
			t.Fatalf("tool messages must not contain extracted image %q; out=%s", imageURL, out)
		}
	}
	if got := messages[2].Get("content").String(); !strings.Contains(got, `"large":9007199254740993`) {
		t.Fatalf("rewritten tool output lost large integer: %q", got)
	}
	if got := messages[3].Get("role").String(); got != "user" {
		t.Fatalf("media message role = %q, want user; out=%s", got, out)
	}
	parts := messages[3].Get("content").Array()
	if len(parts) != 4 {
		t.Fatalf("media content parts = %d, want 4; out=%s", len(parts), out)
	}
	if got := parts[0].Get("text").String(); got != "[Tool output media for call call_a]" {
		t.Fatalf("first attribution = %q", got)
	}
	if got := parts[1].Get("image_url.url").String(); got != "data:image/png;base64,AQID" {
		t.Fatalf("first image URL = %q", got)
	}
	if got := parts[2].Get("text").String(); got != "[Tool output media for call call_b]" {
		t.Fatalf("second attribution = %q", got)
	}
	if got := parts[3].Get("image_url.url").String(); got != "https://example.com/b.png" {
		t.Fatalf("second image URL = %q", got)
	}
	if got := messages[4].Get("content").String(); got != "continue" {
		t.Fatalf("deferred message content = %q, want continue", got)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_MovesBareDataURLToolOutput(t *testing.T) {
	raw := []byte(`{"input":[
		{"type":"custom_tool_call","call_id":"call_image","name":"view_image","input":"{}"},
		{"type":"custom_tool_call_output","call_id":"call_image","output":"data:image/jpeg;base64,BAUG"}
	]}`)

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("vision-model", raw, false)
	if got := gjson.GetBytes(out, "messages.1.content").String(); got != toolOutputMediaMarker {
		t.Fatalf("tool content = %q, want marker; out=%s", got, out)
	}
	if got := gjson.GetBytes(out, "messages.2.content.1.image_url.url").String(); got != "data:image/jpeg;base64,BAUG" {
		t.Fatalf("media URL = %q; out=%s", got, out)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_PreservesMediaFreeToolOutputJSON(t *testing.T) {
	raw := []byte(`{"input":[
		{"type":"function_call","call_id":"call_text","name":"exec","arguments":"{}"},
		{"type":"function_call_output","call_id":"call_text","output":[ { "type": "input_text", "text": "ok" }, {"unknown":true} ]}
	]}`)

	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("text-model", raw, false)
	want := `[ { "type": "input_text", "text": "ok" }, {"unknown":true} ]`
	if got := gjson.GetBytes(out, "messages.1.content").String(); got != want {
		t.Fatalf("tool content = %q, want original JSON %q; out=%s", got, want, out)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_DropsToolChoiceWithoutTools(t *testing.T) {
	tests := []struct {
		name string
		raw  string
	}{
		{name: "missing tools", raw: `{"input":"hello","tool_choice":"auto"}`},
		{name: "null tools", raw: `{"input":"hello","tools":null,"tool_choice":"auto"}`},
		{name: "empty tools", raw: `{"input":"hello","tools":[],"tool_choice":"auto"}`},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("grok-4.5", []byte(tt.raw), false)
			if got := gjson.GetBytes(out, "tool_choice"); got.Exists() {
				t.Fatalf("tool_choice should be omitted when tools are unavailable; out=%s", out)
			}
		})
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_PreservesToolChoiceWithTools(t *testing.T) {
	raw := []byte(`{
		"input":"hello",
		"tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}],
		"tool_choice":"auto"
	}`)
	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("grok-4.5", raw, false)

	if got := gjson.GetBytes(out, "tool_choice").String(); got != "auto" {
		t.Fatalf("tool_choice = %q, want auto; out=%s", got, out)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_IncludesUsageForStreaming(t *testing.T) {
	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions("kimi-k2.6", []byte(`{"input":"hello"}`), true)

	if got := gjson.GetBytes(out, "stream_options.include_usage").Bool(); !got {
		t.Fatalf("stream_options.include_usage = %v, want true; out=%s", got, out)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_PreservesServiceTier(t *testing.T) {
	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions(
		"gpt-5.4-mini",
		[]byte(`{"input":"hello","service_tier":"priority"}`),
		false,
	)

	if got := gjson.GetBytes(out, "service_tier").String(); got != "priority" {
		t.Fatalf("service_tier = %q, want priority; out=%s", got, out)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_PreservesServiceTierForStreamingReasoning(t *testing.T) {
	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions(
		"gpt-5.4-mini",
		[]byte(`{"input":"hello","stream":true,"reasoning":{"effort":"low"},"service_tier":"priority"}`),
		true,
	)

	if got := gjson.GetBytes(out, "stream").Bool(); !got {
		t.Fatalf("stream = %v, want true; out=%s", got, out)
	}
	if got := gjson.GetBytes(out, "stream_options.include_usage").Bool(); !got {
		t.Fatalf("stream_options.include_usage = %v, want true; out=%s", got, out)
	}
	if got := gjson.GetBytes(out, "reasoning_effort").String(); got != "low" {
		t.Fatalf("reasoning_effort = %q, want low; out=%s", got, out)
	}
	if got := gjson.GetBytes(out, "service_tier").String(); got != "priority" {
		t.Fatalf("service_tier = %q, want priority; out=%s", got, out)
	}
}

func TestConvertOpenAIResponsesRequestToOpenAIChatCompletions_DoesNotInventServiceTier(t *testing.T) {
	out := ConvertOpenAIResponsesRequestToOpenAIChatCompletions(
		"gpt-5.4-mini",
		[]byte(`{"input":"hello"}`),
		false,
	)

	if got := gjson.GetBytes(out, "service_tier"); got.Exists() {
		t.Fatalf("service_tier should be absent; out=%s", out)
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
