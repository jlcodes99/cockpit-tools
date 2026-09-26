package main

import (
	"bytes"
	"strings"
	"testing"

	"github.com/tidwall/gjson"
)

// codexAdditionalToolsRequestBody 复刻 Codex 0.156 的工具声明形态：工具藏在
// `input[].additional_tools` 里，namespace 嵌套，freeform 工具用 `custom` 声明。
const codexAdditionalToolsRequestBody = `{
  "model": "gpt-5.6-sol",
  "stream": true,
  "tool_choice": "auto",
  "input": [
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "list files"}]},
    {"type": "additional_tools", "id": "at_1", "role": "developer", "tools": [
      {"type": "namespace", "name": "functions", "description": "", "tools": [
        {"type": "custom", "name": "exec", "description": "Run JavaScript"},
        {"type": "function", "name": "wait", "description": "Wait", "parameters": {"type": "object", "properties": {"cell_id": {"type": "string"}}, "required": ["cell_id"]}}
      ]},
      {"type": "namespace", "name": "collaboration", "description": "", "tools": [
        {"type": "function", "name": "list_agents", "parameters": {"type": "object", "properties": {}}}
      ]}
    ]}
  ]
}`

func TestProviderGatewayHoistAdditionalToolsNoOpWithoutTheItem(t *testing.T) {
	body := []byte(`{"model":"m","input":[{"type":"message","role":"user","content":[]}],"tools":[{"type":"function","name":"keep"}]}`)

	got, freeform := providerGatewayHoistAdditionalTools(body, true)
	if !bytes.Equal(got, body) {
		t.Fatalf("body changed without additional_tools: %s", got)
	}
	if len(freeform) != 0 {
		t.Fatalf("freeform = %v, want empty", freeform)
	}
}

func TestProviderGatewayHoistAdditionalToolsFlattensNamespaces(t *testing.T) {
	got, freeform := providerGatewayHoistAdditionalTools([]byte(codexAdditionalToolsRequestBody), true)

	names := toolNames(t, got, "tools")
	want := []string{"exec", "wait", "list_agents"}
	if strings.Join(names, ",") != strings.Join(want, ",") {
		t.Fatalf("top-level tools = %v, want %v; body=%s", names, want, got)
	}
	if len(freeform) != 1 || !freeform["exec"] {
		t.Fatalf("freeform = %v, want only exec", freeform)
	}

	// additional_tools 项必须从 input 里消失，其余输入项保持原样与原顺序。
	types := inputItemTypes(t, got)
	if strings.Join(types, ",") != "message" {
		t.Fatalf("input item types = %v, want only the message", types)
	}
	if text := gjson.GetBytes(got, "input.0.content.0.text").String(); text != "list files" {
		t.Fatalf("message content lost: %q", text)
	}
}

func TestProviderGatewayHoistAdditionalToolsConvertsFreeformTools(t *testing.T) {
	got, freeform := providerGatewayHoistAdditionalTools([]byte(codexAdditionalToolsRequestBody), true)

	exec := gjson.GetBytes(got, `tools.#(name=="exec")`)
	if exec.Get("type").String() != "function" {
		t.Fatalf("exec type = %q, want function", exec.Get("type").String())
	}
	if arg := exec.Get("parameters.properties.input.type").String(); arg != "string" {
		t.Fatalf("exec input param type = %q, want string", arg)
	}
	if required := exec.Get("parameters.required.0").String(); required != "input" {
		t.Fatalf("exec required = %q, want input", required)
	}
	if desc := exec.Get("description").String(); desc != "Run JavaScript" {
		t.Fatalf("exec description lost: %q", desc)
	}
	if len(freeform) != 1 || !freeform["exec"] {
		t.Fatalf("freeform = %v, want only exec", freeform)
	}
}

// chat_completions 上游的响应转换链路没有 freeform 还原逻辑，那里必须只提升 function 工具。
func TestProviderGatewayHoistAdditionalToolsSkipsFreeformToolsWhenNotIncluded(t *testing.T) {
	got, freeform := providerGatewayHoistAdditionalTools([]byte(codexAdditionalToolsRequestBody), false)

	if names := toolNames(t, got, "tools"); strings.Join(names, ",") != "wait,list_agents" {
		t.Fatalf("top-level tools = %v, want the function tools only", names)
	}
	if len(freeform) != 0 {
		t.Fatalf("freeform = %v, want empty", freeform)
	}
}

func TestProviderGatewayHoistAdditionalToolsKeepsExistingToolsAndDropsDuplicates(t *testing.T) {
	body := []byte(`{"input":[{"type":"additional_tools","tools":[
		{"type":"function","name":"wait","description":"from additional"},
		{"type":"function","name":"fresh","description":"new"}]}],
		"tools":[{"type":"function","name":"wait","description":"from top level"}]}`)

	got, _ := providerGatewayHoistAdditionalTools(body, true)

	if names := toolNames(t, got, "tools"); strings.Join(names, ",") != "wait,fresh" {
		t.Fatalf("top-level tools = %v, want wait then fresh", names)
	}
	if desc := gjson.GetBytes(got, `tools.#(name=="wait").description`).String(); desc != "from top level" {
		t.Fatalf("existing tool was replaced: %q", desc)
	}
}

func TestProviderGatewayHoistAdditionalToolsKeepsNonToolItemsWithoutTools(t *testing.T) {
	// 只有 additional_tools 项、且其中没有任何可提升工具时，仍要把该项摘掉。
	body := []byte(`{"input":[{"type":"additional_tools","tools":[]},{"type":"message","role":"user"}]}`)

	got, freeform := providerGatewayHoistAdditionalTools(body, true)

	if types := inputItemTypes(t, got); strings.Join(types, ",") != "message" {
		t.Fatalf("input item types = %v, want only the message", types)
	}
	if gjson.GetBytes(got, "tools").Exists() {
		t.Fatalf("unexpected tools array: %s", got)
	}
	if len(freeform) != 0 {
		t.Fatalf("freeform = %v, want empty", freeform)
	}
}

func TestProviderGatewayAdditionalToolsResponsePayloadRestoresCustomToolCall(t *testing.T) {
	payload := []byte(`{"type":"response.completed","response":{"output":[
		{"type":"function_call","id":"fc_1","call_id":"c1","name":"exec","arguments":"{\"input\":\"ls -la\"}"},
		{"type":"function_call","id":"fc_2","call_id":"c2","name":"wait","arguments":"{\"cell_id\":\"1\"}"}]}}`)

	got := providerGatewayAdditionalToolsResponsePayload(payload, map[string]bool{"exec": true})

	exec := gjson.GetBytes(got, "response.output.0")
	if exec.Get("type").String() != "custom_tool_call" {
		t.Fatalf("exec type = %q, want custom_tool_call", exec.Get("type").String())
	}
	if input := exec.Get("input").String(); input != "ls -la" {
		t.Fatalf("exec input = %q, want the raw payload", input)
	}
	if exec.Get("arguments").Exists() {
		t.Fatalf("arguments should be dropped: %s", exec.Raw)
	}
	if callID := exec.Get("call_id").String(); callID != "c1" {
		t.Fatalf("call_id = %q, want c1", callID)
	}

	wait := gjson.GetBytes(got, "response.output.1")
	if wait.Get("type").String() != "function_call" {
		t.Fatalf("non-freeform tool was rewritten: %s", wait.Raw)
	}
}

func TestProviderGatewayAdditionalToolsResponsePayloadKeepsUnparsableArguments(t *testing.T) {
	payload := []byte(`{"type":"response.completed","response":{"output":[
		{"type":"function_call","id":"fc_1","call_id":"c1","name":"exec","arguments":"not json"}]}}`)

	got := providerGatewayAdditionalToolsResponsePayload(payload, map[string]bool{"exec": true})

	if input := gjson.GetBytes(got, "response.output.0.input").String(); input != "not json" {
		t.Fatalf("input = %q, want the original text", input)
	}
}

func TestProviderGatewayAdditionalToolsRewriterDropsArgumentDeltasAndRewritesDone(t *testing.T) {
	rewriter := newProviderGatewayAdditionalToolsRewriter(map[string]bool{"exec": true})

	added := []byte("data: {\"type\":\"response.output_item.added\",\"output_index\":1," +
		"\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"name\":\"exec\",\"call_id\":\"c1\"}}\n")
	if got := rewriter.RewriteSSELine(added); got == nil {
		t.Fatal("output_item.added dropped")
	}

	delta := []byte("data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"input\\\":\"}\n")
	if got := rewriter.RewriteSSELine(delta); got != nil {
		t.Fatalf("freeform argument delta should be dropped, got %s", got)
	}

	done := []byte("data: {\"type\":\"response.output_item.done\",\"output_index\":1," +
		"\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"name\":\"exec\",\"call_id\":\"c1\",\"arguments\":\"{\\\"input\\\":\\\"ls\\\"}\"}}\n")
	got := rewriter.RewriteSSELine(done)
	if got == nil {
		t.Fatal("output_item.done dropped")
	}
	if itemType := framePayload(t, got, "item.type").String(); itemType != "custom_tool_call" {
		t.Fatalf("item.type = %q, want custom_tool_call; frame=%s", itemType, got)
	}
	if input := framePayload(t, got, "item.input").String(); input != "ls" {
		t.Fatalf("item.input = %q, want ls; frame=%s", input, got)
	}

	// 非 freeform 工具的增量事件必须原样透传。
	otherDelta := []byte("data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_other\",\"delta\":\"{}\"}\n")
	if got := rewriter.RewriteSSELine(otherDelta); !bytes.Equal(got, otherDelta) {
		t.Fatalf("unrelated delta rewritten: %s", got)
	}
}

func TestProviderGatewayAdditionalToolsRewriterLeavesNonDataLinesAlone(t *testing.T) {
	rewriter := newProviderGatewayAdditionalToolsRewriter(map[string]bool{"exec": true})

	for _, line := range [][]byte{
		[]byte("event: response.output_item.done\n"),
		[]byte("\n"),
		[]byte("data: [DONE]\n"),
	} {
		if got := rewriter.RewriteSSELine(line); !bytes.Equal(got, line) {
			t.Fatalf("line %q rewritten to %q", line, got)
		}
	}
}

func toolNames(t *testing.T, body []byte, path string) []string {
	t.Helper()
	tools := gjson.GetBytes(body, path)
	if !tools.IsArray() {
		t.Fatalf("%s is not an array: %s", path, body)
	}
	names := make([]string, 0, len(tools.Array()))
	for _, tool := range tools.Array() {
		names = append(names, tool.Get("name").String())
	}
	return names
}

func inputItemTypes(t *testing.T, body []byte) []string {
	t.Helper()
	items := gjson.GetBytes(body, "input")
	if !items.IsArray() {
		t.Fatalf("input is not an array: %s", body)
	}
	types := make([]string, 0, len(items.Array()))
	for _, item := range items.Array() {
		types = append(types, item.Get("type").String())
	}
	return types
}
