package messages

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

func TestMessagesHandler_DeepSeekChatAndMessagesThinkingMatrix(t *testing.T) {
	tests := []struct {
		name           string
		serviceType    string
		responseBody   string
		wantUpstream   func(t *testing.T, body []byte)
		wantDownstream func(t *testing.T, body []byte)
	}{
		{
			name:         "messages_to_deepseek_chat",
			serviceType:  "openai",
			responseBody: `{"id":"chatcmpl_ds","choices":[{"message":{"role":"assistant","reasoning_content":"chat reasoning","content":"chat text"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}`,
			wantUpstream: func(t *testing.T, body []byte) {
				var req map[string]interface{}
				if err := json.Unmarshal(body, &req); err != nil {
					t.Fatalf("unmarshal upstream request: %v", err)
				}
				messages, ok := req["messages"].([]interface{})
				if !ok || len(messages) < 2 {
					t.Fatalf("messages shape invalid: %s", string(body))
				}
				assistant, ok := messages[1].(map[string]interface{})
				if !ok {
					t.Fatalf("assistant message shape invalid: %s", string(body))
				}
				if got := assistant["reasoning_content"]; got != "previous thinking" {
					t.Fatalf("reasoning_content = %v, want previous thinking; body=%s", got, string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"type":"thinking"`)) || !bytes.Contains(body, []byte(`"thinking":"chat reasoning"`)) {
					t.Fatalf("expected Claude thinking block from reasoning_content, got %s", string(body))
				}
			},
		},
		{
			name:         "messages_to_deepseek_messages",
			serviceType:  "claude",
			responseBody: `{"id":"msg_ds","type":"message","role":"assistant","content":[{"type":"thinking","thinking":"messages thinking","signature":"sig_ds"},{"type":"text","text":"messages text"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":2}}`,
			wantUpstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"type":"thinking"`)) || !bytes.Contains(body, []byte(`"signature":"sig_prev"`)) {
					t.Fatalf("expected upstream Claude thinking+signature, got %s", string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"thinking":"messages thinking"`)) || !bytes.Contains(body, []byte(`"signature":"sig_ds"`)) {
					t.Fatalf("expected downstream Claude thinking+signature, got %s", string(body))
				}
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var captured []byte
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				body, err := io.ReadAll(r.Body)
				if err != nil {
					t.Fatalf("read upstream request: %v", err)
				}
				captured = body
				w.Header().Set("Content-Type", "application/json")
				w.WriteHeader(http.StatusOK)
				_, _ = w.Write([]byte(tt.responseBody))
			}))
			defer upstream.Close()

			router := newMessagesTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			})

			reqBody := `{"model":"deepseek-v4-pro","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]},{"role":"assistant","content":[{"type":"thinking","thinking":"previous thinking","signature":"sig_prev"},{"type":"text","text":"previous text"}]}]}`
			w := performMessagesHandlerRequest(t, router, reqBody)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
			}
			tt.wantUpstream(t, captured)
			tt.wantDownstream(t, w.Body.Bytes())
		})
	}
}
