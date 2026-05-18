package providers

import (
	"net/http"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

func TestMessagesEntry_ResponseMatrix_AllFourUpstreams(t *testing.T) {
	tests := []struct {
		name         string
		provider     Provider
		body         string
		expectText   bool
		expectTool   bool
		stopReason   string
		inputTokens  int
		outputTokens int
	}{
		{
			name:         "messages_from_claude",
			provider:     &ClaudeProvider{},
			body:         `{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{"input_tokens":11,"output_tokens":7}}`,
			expectText:   true,
			stopReason:   "end_turn",
			inputTokens:  11,
			outputTokens: 7,
		},
		{
			name:         "messages_from_openai",
			provider:     &OpenAIProvider{},
			body:         `{"id":"chatcmpl_1","choices":[{"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":13,"completion_tokens":5,"total_tokens":18}}`,
			expectText:   true,
			stopReason:   "end_turn",
			inputTokens:  13,
			outputTokens: 5,
		},
		{
			name:         "messages_from_gemini",
			provider:     &GeminiProvider{},
			body:         `{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":17,"candidatesTokenCount":3,"totalTokenCount":20}}`,
			expectText:   true,
			stopReason:   "end_turn",
			inputTokens:  17,
			outputTokens: 3,
		},
		{
			name:         "messages_from_responses",
			provider:     &ResponsesProvider{},
			body:         `{"id":"resp_1","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}],"usage":{"input_tokens":19,"output_tokens":9,"total_tokens":28}}`,
			expectText:   true,
			stopReason:   "end_turn",
			inputTokens:  19,
			outputTokens: 9,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			providerResp := &types.ProviderResponse{
				StatusCode: http.StatusOK,
				Headers:    map[string][]string{"Content-Type": {"application/json"}},
				Body:       []byte(tt.body),
			}

			claudeResp, err := tt.provider.ConvertToClaudeResponse(providerResp)
			if err != nil {
				t.Fatalf("ConvertToClaudeResponse() err = %v", err)
			}
			if claudeResp == nil {
				t.Fatal("response is nil")
			}
			if len(claudeResp.Content) == 0 {
				t.Fatalf("content is empty: %#v", claudeResp)
			}
			if claudeResp.StopReason != tt.stopReason {
				t.Fatalf("stop_reason = %q, want %q", claudeResp.StopReason, tt.stopReason)
			}
			if claudeResp.Usage == nil {
				t.Fatalf("usage is nil: %#v", claudeResp)
			}
			if claudeResp.Usage.InputTokens != tt.inputTokens {
				t.Fatalf("input_tokens = %d, want %d", claudeResp.Usage.InputTokens, tt.inputTokens)
			}
			if claudeResp.Usage.OutputTokens != tt.outputTokens {
				t.Fatalf("output_tokens = %d, want %d", claudeResp.Usage.OutputTokens, tt.outputTokens)
			}

			first := claudeResp.Content[0]
			if tt.expectText && first.Type != "text" {
				t.Fatalf("content[0].type = %q, want text", first.Type)
			}
			if tt.expectText && first.Text != "hi" {
				t.Fatalf("content[0].text = %q, want hi", first.Text)
			}
		})
	}
}

func TestMessagesEntry_ResponseMatrix_ToolUseNormalization(t *testing.T) {
	tests := []struct {
		name       string
		provider   Provider
		body       string
		expectID   string
		expectName string
		wantNoPage bool
	}{
		{
			name:       "openai_tool_call_to_tool_use",
			provider:   &OpenAIProvider{},
			body:       `{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"call_openai","type":"function","function":{"name":"Read","arguments":"{\"file_path\":\"/tmp/x\",\"pages\":\"\"}"}}]},"finish_reason":"tool_calls"}]}`,
			expectID:   "call_openai",
			expectName: "Read",
			wantNoPage: true,
		},
		{
			name:       "gemini_function_call_to_tool_use",
			provider:   &GeminiProvider{},
			body:       `{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"search_docs","args":{"query":"responses"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1}}`,
			expectName: "search_docs",
		},
		{
			name:       "responses_function_call_to_tool_use",
			provider:   &ResponsesProvider{},
			body:       `{"id":"resp_tool","status":"completed","output":[{"type":"function_call","call_id":"call_resp","name":"Read","arguments":"{\"file_path\":\"/tmp/y\",\"pages\":\"\"}"}],"usage":{"input_tokens":1,"output_tokens":1}}`,
			expectID:   "call_resp",
			expectName: "Read",
			wantNoPage: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			providerResp := &types.ProviderResponse{
				StatusCode: http.StatusOK,
				Headers:    map[string][]string{"Content-Type": {"application/json"}},
				Body:       []byte(tt.body),
			}

			claudeResp, err := tt.provider.ConvertToClaudeResponse(providerResp)
			if err != nil {
				t.Fatalf("ConvertToClaudeResponse() err = %v", err)
			}
			if len(claudeResp.Content) == 0 {
				t.Fatalf("content is empty: %#v", claudeResp)
			}

			var toolBlock *types.ClaudeContent
			for i := range claudeResp.Content {
				if claudeResp.Content[i].Type == "tool_use" {
					toolBlock = &claudeResp.Content[i]
					break
				}
			}
			if toolBlock == nil {
				t.Fatalf("expected tool_use block, got %#v", claudeResp.Content)
			}
			if tt.expectID != "" && toolBlock.ID != tt.expectID {
				t.Fatalf("tool id = %q, want %q", toolBlock.ID, tt.expectID)
			}
			if toolBlock.Name != tt.expectName {
				t.Fatalf("tool name = %q, want %q", toolBlock.Name, tt.expectName)
			}
			if tt.wantNoPage {
				input, ok := toolBlock.Input.(map[string]interface{})
				if !ok {
					t.Fatalf("tool input type = %T, want map[string]interface{}", toolBlock.Input)
				}
				if _, exists := input["pages"]; exists {
					t.Fatalf("tool input pages exists = true, want false; input=%#v", input)
				}
			}
		})
	}
}
