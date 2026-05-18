package providers

import (
	"net/http"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

func TestOpenAIProvider_ConvertToClaudeResponse_StripsEmptyReadPages(t *testing.T) {
	provider := &OpenAIProvider{}
	providerResp := &types.ProviderResponse{
		StatusCode: http.StatusOK,
		Headers:    map[string][]string{"Content-Type": {"application/json"}},
		Body: []byte(`{
			"choices":[
				{
					"message":{
						"role":"assistant",
						"tool_calls":[
							{
								"id":"call_1",
								"type":"function",
								"function":{"name":"Read","arguments":"{\"file_path\":\"/tmp/x\",\"pages\":\"\"}"}
							}
						]
					}
				}
			]
		}`),
	}

	claudeResp, err := provider.ConvertToClaudeResponse(providerResp)
	if err != nil {
		t.Fatalf("ConvertToClaudeResponse() err = %v", err)
	}
	if len(claudeResp.Content) != 1 {
		t.Fatalf("len(Content) = %d, want 1", len(claudeResp.Content))
	}
	block := claudeResp.Content[0]
	if block.Type != "tool_use" || block.Name != "Read" {
		t.Fatalf("content[0] = %#v, want tool_use Read", block)
	}
	input, ok := block.Input.(map[string]interface{})
	if !ok {
		t.Fatalf("content[0].Input type = %T, want map[string]interface{}", block.Input)
	}
	if _, exists := input["pages"]; exists {
		t.Fatalf("content[0].Input.pages exists = true, want false; input=%v", input)
	}
	if input["file_path"] != "/tmp/x" {
		t.Fatalf("content[0].Input.file_path = %v, want /tmp/x", input["file_path"])
	}
}
