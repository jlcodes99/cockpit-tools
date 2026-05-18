package converters

import (
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

func TestOpenAIChatConverter_ReplaysCustomToolCallInput(t *testing.T) {
	req := &types.ResponsesRequest{
		Model: "gpt-4o",
		Tools: []map[string]interface{}{{
			"type": "custom",
			"name": "apply_patch",
		}},
		Input: []types.ResponsesItem{
			{
				Type:   "custom_tool_call",
				CallID: "call_1",
				Name:   "apply_patch",
				Input:  "*** Begin Patch\n*** Add File: docs/test.md\n+# Test\n*** End Patch",
			},
			{
				Type:   "custom_tool_call_output",
				CallID: "call_1",
				Output: "ok",
			},
		},
	}

	converted, err := (&OpenAIChatConverter{}).ToProviderRequest(&session.Session{}, req)
	if err != nil {
		t.Fatal(err)
	}
	body := converted.(map[string]interface{})
	messages := body["messages"].([]map[string]interface{})
	if len(messages) != 2 {
		t.Fatalf("messages len = %d", len(messages))
	}
	toolCalls := messages[0]["tool_calls"].([]map[string]interface{})
	function := toolCalls[0]["function"].(map[string]interface{})
	if function["name"] != "apply_patch_add_file" {
		t.Fatalf("function name = %v", function["name"])
	}
	if got := function["arguments"].(string); got != `{"content":"# Test\n","path":"docs/test.md"}` {
		t.Fatalf("arguments = %s", got)
	}
	if messages[1]["role"] != "tool" || messages[1]["tool_call_id"] != "call_1" {
		t.Fatalf("tool output message = %#v", messages[1])
	}
}
