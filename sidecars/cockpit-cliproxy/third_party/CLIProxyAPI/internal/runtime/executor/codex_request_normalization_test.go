package executor

import (
	"testing"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
)

func TestNormalizeCodexInstructionsUsesModelBaseInstructions(t *testing.T) {
	for _, body := range [][]byte{
		[]byte(`{"model":"gpt-5.6-sol"}`),
		[]byte(`{"instructions":null}`),
		[]byte(`{"instructions":"  \n\t"}`),
	} {
		got := normalizeCodexInstructions(body, "gpt-5.6-sol")
		if instructions := gjson.GetBytes(got, "instructions").String(); len(instructions) < 100 {
			t.Fatalf("expected non-empty model base instructions, got %q from %s", instructions, body)
		}
	}

	body := []byte(`{"instructions":"keep me"}`)
	if got := gjson.GetBytes(normalizeCodexInstructions(body, "gpt-5.6-sol"), "instructions").String(); got != "keep me" {
		t.Fatalf("explicit instructions = %q, want keep me", got)
	}
}

func TestNormalizeCodexInputNamespacesForOAuth(t *testing.T) {
	body := []byte(`{"input":[{"type":"function_call","namespace":"functions"},{"type":"custom_tool_call","namespace":"custom"},{"type":"tool_call","namespace":"tool"},{"type":"mcp_tool_call","namespace":"mcp"},{"type":"message","namespace":"stale"},{"type":"reasoning","namespace":"stale"}]}`)
	got := normalizeCodexInputNamespaces(body, &cliproxyauth.Auth{}, false)
	items := gjson.GetBytes(got, "input").Array()

	for index := 0; index < 4; index++ {
		if !items[index].Get("namespace").Exists() {
			t.Fatalf("call item %d namespace was removed: %s", index, got)
		}
	}
	for index := 4; index < 6; index++ {
		if items[index].Get("namespace").Exists() {
			t.Fatalf("non-call item %d namespace was preserved: %s", index, got)
		}
	}
}

func TestNormalizeCodexInputNamespacesRemovesAllForAPIKeyAndCompact(t *testing.T) {
	body := []byte(`{"input":[{"type":"function_call","namespace":"functions"},{"type":"message","namespace":"stale"}]}`)
	apiKeyAuth := &cliproxyauth.Auth{Attributes: map[string]string{"api_key": "test"}}
	for name, got := range map[string][]byte{
		"api-key": normalizeCodexInputNamespaces(body, apiKeyAuth, false),
		"compact": normalizeCodexInputNamespaces(body, &cliproxyauth.Auth{}, true),
	} {
		for _, item := range gjson.GetBytes(got, "input").Array() {
			if item.Get("namespace").Exists() {
				t.Fatalf("%s request retained namespace: %s", name, got)
			}
		}
	}
}
