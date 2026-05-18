package responses

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
)

func TestResponsesHandler_DeepSeekChatAndMessagesThinkingMatrix(t *testing.T) {
	tests := []struct {
		name           string
		serviceType    string
		responseBody   string
		wantUpstream   func(t *testing.T, body []byte)
		wantDownstream func(t *testing.T, body []byte)
	}{
		{
			name:         "responses_to_deepseek_chat",
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
				assistant, ok := messages[0].(map[string]interface{})
				if !ok {
					t.Fatalf("assistant reasoning message shape invalid: %s", string(body))
				}
				if got := assistant["reasoning_content"]; got != "previous reasoning" {
					t.Fatalf("reasoning_content = %v, want previous reasoning; body=%s", got, string(body))
				}
				if got := chatTextContent(assistant["content"]); got != "previous text" {
					t.Fatalf("content = %v, want previous text in same assistant message; body=%s", got, string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"type":"reasoning"`)) || !bytes.Contains(body, []byte(`"text":"chat reasoning"`)) {
					t.Fatalf("expected Responses reasoning item from reasoning_content, got %s", string(body))
				}
			},
		},
		{
			name:         "responses_to_deepseek_messages",
			serviceType:  "claude",
			responseBody: `{"id":"msg_ds","type":"message","role":"assistant","content":[{"type":"thinking","thinking":"messages thinking","signature":"sig_ds"},{"type":"text","text":"messages text"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":2}}`,
			wantUpstream: func(t *testing.T, body []byte) {
				var req struct {
					Messages []struct {
						Role    string                   `json:"role"`
						Content []map[string]interface{} `json:"content"`
					} `json:"messages"`
				}
				if err := json.Unmarshal(body, &req); err != nil {
					t.Fatalf("unmarshal upstream request: %v", err)
				}
				if len(req.Messages) < 1 || req.Messages[0].Role != "assistant" || len(req.Messages[0].Content) < 2 {
					t.Fatalf("expected merged assistant thinking+text message, got %s", string(body))
				}
				if req.Messages[0].Content[0]["type"] != "thinking" || req.Messages[0].Content[0]["thinking"] != "previous reasoning" {
					t.Fatalf("expected first content block to be thinking, got %s", string(body))
				}
				if req.Messages[0].Content[1]["type"] != "text" || req.Messages[0].Content[1]["text"] != "previous text" {
					t.Fatalf("expected second content block to be text, got %s", string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"type":"reasoning"`)) || !bytes.Contains(body, []byte(`"text":"messages thinking"`)) {
					t.Fatalf("expected Responses reasoning item from Claude thinking, got %s", string(body))
				}
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			sessionManager := session.NewSessionManager(time.Hour, 100, 100000)
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

			router := newResponsesTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			}, sessionManager)

			reqBody := `{"model":"deepseek-v4-pro","input":[{"type":"reasoning","status":"completed","summary":[{"type":"summary_text","text":"previous reasoning"}]},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"previous text"}]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}],"store":false}`
			w := performResponsesHandlerRequest(t, router, reqBody)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
			}
			tt.wantUpstream(t, captured)
			tt.wantDownstream(t, w.Body.Bytes())
		})
	}
}

func chatTextContent(content interface{}) string {
	switch v := content.(type) {
	case string:
		return v
	case []interface{}:
		if len(v) == 0 {
			return ""
		}
		part, ok := v[0].(map[string]interface{})
		if !ok {
			return ""
		}
		text, _ := part["text"].(string)
		return text
	default:
		return ""
	}
}

func TestResponsesHandler_DeepSeekChatAndMessagesStreamThinkingMatrix(t *testing.T) {
	tests := []struct {
		name         string
		serviceType  string
		responseBody string
		want         []string
	}{
		{
			name:        "responses_stream_to_deepseek_chat",
			serviceType: "openai",
			responseBody: strings.Join([]string{
				`data: {"id":"chatcmpl_ds","object":"chat.completion.chunk","created":123,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{"reasoning_content":"chat reasoning"},"finish_reason":null}]}`,
				`data: {"id":"chatcmpl_ds","object":"chat.completion.chunk","created":123,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{"content":"chat text"},"finish_reason":null}]}`,
				`data: {"id":"chatcmpl_ds","object":"chat.completion.chunk","created":123,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}`,
				`data: [DONE]`,
				``,
			}, "\n"),
			want: []string{
				`response.reasoning_summary_text.delta`,
				`"text":"chat reasoning"`,
				`"delta":"chat text"`,
				`"type":"response.completed"`,
			},
		},
		{
			name:        "responses_stream_to_deepseek_messages_tool_use",
			serviceType: "claude",
			responseBody: strings.Join([]string{
				`event: message_start`,
				`data: {"type":"message_start","message":{"id":"msg_ds","type":"message","role":"assistant","model":"deepseek-v4-pro","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}`,
				``,
				`event: content_block_start`,
				`data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_read","name":"Read","input":{}}}`,
				``,
				`event: content_block_delta`,
				`data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\""}}`,
				``,
				`event: content_block_delta`,
				`data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":":\"/tmp/a\"}"}}`,
				``,
				`event: content_block_stop`,
				`data: {"type":"content_block_stop","index":0}`,
				``,
				`event: message_delta`,
				`data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"input_tokens":1,"output_tokens":2}}`,
				``,
				`event: message_stop`,
				`data: {"type":"message_stop"}`,
				``,
			}, "\n"),
			want: []string{
				`response.function_call_arguments.delta`,
				`response.function_call_arguments.done`,
				`"type":"function_call"`,
				`"call_id":"call_read"`,
				`"name":"Read"`,
				`"arguments":"{\"file_path\":\"/tmp/a\"}"`,
				`"type":"response.completed"`,
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			sessionManager := session.NewSessionManager(time.Hour, 100, 100000)
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "text/event-stream")
				w.WriteHeader(http.StatusOK)
				_, _ = w.Write([]byte(tt.responseBody))
			}))
			defer upstream.Close()

			router := newResponsesTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			}, sessionManager)

			reqBody := `{"model":"deepseek-v4-pro","stream":true,"input":"hello","store":false}`
			w := performResponsesHandlerRequest(t, router, reqBody)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
			}
			body := w.Body.String()
			for _, want := range tt.want {
				if !strings.Contains(body, want) {
					t.Fatalf("expected %q in stream body, got %s", want, body)
				}
			}
		})
	}
}
