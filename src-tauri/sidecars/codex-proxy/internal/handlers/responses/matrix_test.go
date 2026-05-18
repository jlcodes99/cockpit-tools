package responses

import (
	"encoding/json"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/providers"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

func TestResponsesEntry_ResponseMatrix_AllFourUpstreams(t *testing.T) {
	provider := &providers.ResponsesProvider{}
	tests := []struct {
		name         string
		upstreamType string
		body         string
	}{
		{"responses_from_responses", "responses", `{"id":"resp_1","model":"gpt-5","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}`},
		{"responses_from_claude", "claude", `{"id":"msg_1","model":"claude-3-5-sonnet","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":1,"output_tokens":1}}`},
		{"responses_from_openai", "openai", `{"id":"chatcmpl_1","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`},
		{"responses_from_gemini", "gemini", `{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}`},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			resp, err := provider.ConvertToResponsesResponse(&types.ProviderResponse{Body: []byte(tt.body)}, tt.upstreamType, "")
			if err != nil {
				t.Fatalf("ConvertToResponsesResponse() err = %v", err)
			}
			if resp == nil {
				t.Fatal("response is nil")
			}
			b, _ := json.Marshal(resp)
			var m map[string]interface{}
			_ = json.Unmarshal(b, &m)
			if _, ok := m["output"]; !ok {
				t.Fatalf("expected output field in responses response, got %#v", m)
			}
		})
	}
}
