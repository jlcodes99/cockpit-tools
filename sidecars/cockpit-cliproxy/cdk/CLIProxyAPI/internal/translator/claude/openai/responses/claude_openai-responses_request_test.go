package responses

import (
	"bytes"
	"context"
	"testing"

	"github.com/tidwall/gjson"
)

func TestConvertOpenAIResponsesRequestToClaudeToolOutputsAndAdditionalTools(t *testing.T) {
	input := []byte(`{
		"model":"claude-test",
		"tools":[{"type":"function","name":"top","parameters":{"type":"object"}}],
		"input":[
			{"type":"additional_tools","tools":[
				{"type":"function","name":"extra","parameters":{"type":"object","properties":{"q":{"type":"string"}}}},
				{"type":"custom","name":"patch","description":"Apply a patch"}
			]},
			{"type":"function_call","call_id":"call_a","name":"top","arguments":"{}"},
			{"type":"custom_tool_call","call_id":"call_b","name":"patch","input":"diff"},
			{"type":"function_call_output","call_id":"call_a","output":[
				{"type":"input_text","text":"loaded"},
				{"type":"input_image","image_url":"data:image/png;base64,YQ=="}
			]},
			{"type":"custom_tool_call_output","call_id":"call_b","output":[
				{"type":"image_url","image_url":{"url":"https://example.com/result.png"}}
			]}
		]
	}`)

	out := ConvertOpenAIResponsesRequestToClaude("claude-test", input, false)
	if !gjson.ValidBytes(out) {
		t.Fatalf("invalid output: %s", out)
	}

	tools := gjson.GetBytes(out, "tools")
	for _, name := range []string{"top", "extra", "patch"} {
		if !tools.Get(`#(name=="` + name + `")`).Exists() {
			t.Fatalf("tool %q missing: %s", name, tools.Raw)
		}
	}
	if got := tools.Get(`#(name=="patch").input_schema.properties.input.type`).String(); got != "string" {
		t.Fatalf("custom tool input schema = %q, want string", got)
	}

	messages := gjson.GetBytes(out, "messages")
	if got := len(messages.Array()); got != 2 {
		t.Fatalf("messages count = %d, want 2: %s", got, messages.Raw)
	}
	if got := messages.Get("0.role").String(); got != "assistant" {
		t.Fatalf("first role = %q, want assistant", got)
	}
	if got := messages.Get("0.content.#(type==\"tool_use\")#").Array(); len(got) != 2 {
		t.Fatalf("tool_use count = %d, want 2: %s", len(got), messages.Get("0.content").Raw)
	}
	if got := messages.Get("0.content.#(id==\"call_b\").input.input").String(); got != "diff" {
		t.Fatalf("custom input = %q, want diff", got)
	}
	if got := messages.Get("1.content.#(tool_use_id==\"call_a\").content.0.text").String(); got != "loaded" {
		t.Fatalf("text tool result = %q, want loaded", got)
	}
	if got := messages.Get("1.content.#(tool_use_id==\"call_a\").content.1.source.data").String(); got != "YQ==" {
		t.Fatalf("base64 tool result = %q, want YQ==", got)
	}
	if got := messages.Get("1.content.#(tool_use_id==\"call_b\").content.0.source.url").String(); got != "https://example.com/result.png" {
		t.Fatalf("URL tool result = %q", got)
	}
}

func TestConvertOpenAIResponsesRequestToClaudeEmptyOutputAndPairing(t *testing.T) {
	input := []byte(`{"input":[
		{"type":"message","role":"user","content":[{"type":"input_text","text":"run"}]},
		{"type":"function_call","call_id":"call_a","name":"exec","arguments":"{}"},
		{"type":"function_call","call_id":"call_dangling","name":"exec","arguments":"{}"},
		{"type":"message","role":"developer","content":[{"type":"input_text","text":"approved"}]},
		{"type":"function_call_output","call_id":"call_a","output":[]},
		{"type":"function_call_output","call_id":"call_orphan","output":"ignored"}
	]}`)

	out := ConvertOpenAIResponsesRequestToClaude("claude-test", input, false)
	messages := gjson.GetBytes(out, "messages")
	var toolUseIndex = -1
	messages.ForEach(func(index, message gjson.Result) bool {
		if message.Get("content.#(id==\"call_dangling\")").Exists() {
			t.Fatalf("dangling tool_use was retained: %s", message.Raw)
		}
		if message.Get("content.#(tool_use_id==\"call_orphan\")").Exists() {
			t.Fatalf("orphan tool_result was retained: %s", message.Raw)
		}
		if message.Get("content.#(id==\"call_a\")").Exists() {
			toolUseIndex = int(index.Int())
		}
		return true
	})
	if toolUseIndex < 0 || toolUseIndex+1 >= len(messages.Array()) {
		t.Fatalf("paired tool_use missing: %s", messages.Raw)
	}
	result := messages.Get(string(rune('0'+toolUseIndex+1)) + `.content.#(tool_use_id=="call_a")`)
	if !result.Exists() {
		t.Fatalf("tool_result is not immediately after tool_use: %s", messages.Raw)
	}
	if got := result.Get("content").String(); got != "(empty)" {
		t.Fatalf("empty output = %q, want (empty)", got)
	}
}

func TestConvertOpenAIResponsesRequestToClaudeNamespaceHistory(t *testing.T) {
	input := []byte(`{
		"tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"send_message","parameters":{"type":"object"}}]}],
		"input":[
			{"type":"function_call","namespace":"collaboration","call_id":"call_a","name":"send_message","arguments":"{}"},
			{"type":"function_call_output","call_id":"call_a","output":"ok"}
		]
	}`)
	out := ConvertOpenAIResponsesRequestToClaude("claude-test", input, false)
	if got := gjson.GetBytes(out, `messages.0.content.#(id=="call_a").name`).String(); got != "collaboration__send_message" {
		t.Fatalf("namespace history name = %q", got)
	}
	if !gjson.GetBytes(out, `tools.#(name=="collaboration__send_message")`).Exists() {
		t.Fatalf("flattened namespace tool missing: %s", out)
	}
}

func TestClaudeResponsesNamespaceRestoration(t *testing.T) {
	request := []byte(`{"input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"send_message"}]}]}]}`)
	streamChunks := [][]byte{
		[]byte(`data: {"type":"message_start","message":{"id":"resp_1","usage":{"input_tokens":1}}}`),
		[]byte(`data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_1","name":"collaboration__send_message"}}`),
		[]byte(`data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}`),
		[]byte(`data: {"type":"content_block_stop","index":0}`),
	}
	var state any
	var events [][]byte
	for _, chunk := range streamChunks {
		events = append(events, ConvertClaudeResponseToOpenAIResponses(context.Background(), "claude-test", request, nil, chunk, &state)...)
	}
	joined := bytes.Join(events, []byte("\n"))
	if !bytes.Contains(joined, []byte(`"name":"send_message","namespace":"collaboration"`)) {
		t.Fatalf("stream namespace not restored: %s", joined)
	}

	nonStreamInput := bytes.Join(streamChunks, []byte("\n"))
	nonStream := ConvertClaudeResponseToOpenAIResponsesNonStream(context.Background(), "claude-test", request, nil, nonStreamInput, nil)
	if got := gjson.GetBytes(nonStream, "output.0.name").String(); got != "send_message" {
		t.Fatalf("non-stream name = %q", got)
	}
	if got := gjson.GetBytes(nonStream, "output.0.namespace").String(); got != "collaboration" {
		t.Fatalf("non-stream namespace = %q", got)
	}
}
