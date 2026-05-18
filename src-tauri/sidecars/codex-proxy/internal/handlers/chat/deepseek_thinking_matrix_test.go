package chat

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

func TestChatHandler_DeepSeekChatAndMessagesThinkingMatrix(t *testing.T) {
	tests := []struct {
		name           string
		serviceType    string
		responseBody   string
		wantUpstream   func(t *testing.T, body []byte)
		wantDownstream func(t *testing.T, body []byte)
	}{
		{
			name:         "chat_to_deepseek_chat",
			serviceType:  "openai",
			responseBody: `{"id":"chatcmpl_ds","object":"chat.completion","choices":[{"message":{"role":"assistant","reasoning_content":"chat reasoning","content":"chat text"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}`,
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
				if got := assistant["reasoning_content"]; got != "previous reasoning" {
					t.Fatalf("reasoning_content = %v, want previous reasoning; body=%s", got, string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"reasoning_content":"chat reasoning"`)) {
					t.Fatalf("expected OpenAI reasoning_content passthrough, got %s", string(body))
				}
			},
		},
		{
			name:         "chat_to_deepseek_messages",
			serviceType:  "claude",
			responseBody: `{"id":"msg_ds","type":"message","role":"assistant","content":[{"type":"thinking","thinking":"messages thinking","signature":"sig_ds"},{"type":"text","text":"messages text"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":2}}`,
			wantUpstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"type":"thinking"`)) || !bytes.Contains(body, []byte(`"thinking":"previous reasoning"`)) {
					t.Fatalf("expected converted Claude thinking block, got %s", string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"reasoning_content":"messages thinking"`)) {
					t.Fatalf("expected Claude thinking converted to OpenAI reasoning_content, got %s", string(body))
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

			router := newChatTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			})

			reqBody := `{"model":"deepseek-v4-pro","messages":[{"role":"user","content":"hello"},{"role":"assistant","reasoning_content":"previous reasoning","content":"previous text"}]}`
			w := performChatHandlerRequest(t, router, reqBody)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
			}
			tt.wantUpstream(t, captured)
			tt.wantDownstream(t, w.Body.Bytes())
		})
	}
}

func TestChatHandler_DeepSeekMessagesStreamThinkingToReasoningContent(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(strings.Join([]string{
			`event: message_start`,
			`data: {"type":"message_start","message":{"id":"msg_ds","type":"message","role":"assistant","model":"deepseek-v4-pro","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}`,
			``,
			`event: content_block_start`,
			`data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}`,
			``,
			`event: content_block_delta`,
			`data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"messages thinking"}}`,
			``,
			`event: content_block_stop`,
			`data: {"type":"content_block_stop","index":0}`,
			``,
			`event: content_block_start`,
			`data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}`,
			``,
			`event: content_block_delta`,
			`data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"messages text"}}`,
			``,
			`event: content_block_stop`,
			`data: {"type":"content_block_stop","index":1}`,
			``,
			`event: message_delta`,
			`data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"input_tokens":1,"output_tokens":2}}`,
			``,
			`event: message_stop`,
			`data: {"type":"message_stop"}`,
			``,
		}, "\n")))
	}))
	defer upstream.Close()

	router := newChatTestRouter(t, config.UpstreamConfig{
		Name:        "chat_stream_to_deepseek_messages",
		BaseURL:     upstream.URL,
		APIKeys:     []string{"sk-test"},
		ServiceType: "claude",
		Status:      "active",
	})

	w := performChatHandlerRequest(t, router, `{"model":"deepseek-v4-pro","stream":true,"messages":[{"role":"user","content":"hello"}]}`)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
	}
	body := w.Body.String()
	if !strings.Contains(body, `"reasoning_content":"messages thinking"`) {
		t.Fatalf("expected stream reasoning_content from thinking_delta, got %s", body)
	}
	if !strings.Contains(body, `"content":"messages text"`) {
		t.Fatalf("expected stream text content, got %s", body)
	}
}
