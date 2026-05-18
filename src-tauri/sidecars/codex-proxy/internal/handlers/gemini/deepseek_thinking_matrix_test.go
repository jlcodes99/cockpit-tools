package gemini

import (
	"bytes"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

func TestGeminiHandler_DeepSeekChatAndMessagesThinkingMatrix(t *testing.T) {
	tests := []struct {
		name           string
		serviceType    string
		responseBody   string
		wantUpstream   func(t *testing.T, body []byte)
		wantDownstream func(t *testing.T, body []byte)
	}{
		{
			name:         "gemini_to_deepseek_chat",
			serviceType:  "openai",
			responseBody: `{"id":"chatcmpl_ds","object":"chat.completion","choices":[{"message":{"role":"assistant","reasoning_content":"chat reasoning","content":"chat text"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}`,
			wantUpstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"reasoning_content":"previous reasoning"`)) {
					t.Fatalf("expected upstream OpenAI reasoning_content, got %s", string(body))
				}
				if !bytes.Contains(body, []byte(`"content":"previous text"`)) {
					t.Fatalf("expected upstream OpenAI assistant text, got %s", string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"text":"chat reasoning"`)) || !bytes.Contains(body, []byte(`"thought":true`)) {
					t.Fatalf("expected Gemini thought part from reasoning_content, got %s", string(body))
				}
				if !bytes.Contains(body, []byte(`"text":"chat text"`)) {
					t.Fatalf("expected Gemini text part, got %s", string(body))
				}
			},
		},
		{
			name:         "gemini_to_deepseek_messages",
			serviceType:  "claude",
			responseBody: `{"id":"msg_ds","type":"message","role":"assistant","content":[{"type":"thinking","thinking":"messages thinking","signature":"sig_ds"},{"type":"text","text":"messages text"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":2}}`,
			wantUpstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"type":"thinking"`)) || !bytes.Contains(body, []byte(`"thinking":"previous reasoning"`)) {
					t.Fatalf("expected upstream Claude thinking block, got %s", string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				if !bytes.Contains(body, []byte(`"text":"messages thinking"`)) || !bytes.Contains(body, []byte(`"thought":true`)) {
					t.Fatalf("expected Gemini thought part from Claude thinking, got %s", string(body))
				}
				if !bytes.Contains(body, []byte(`"text":"messages text"`)) {
					t.Fatalf("expected Gemini text part, got %s", string(body))
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

			router := newGeminiTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			})

			reqBody := `{"contents":[{"role":"model","parts":[{"text":"previous reasoning","thought":true},{"text":"previous text"}]},{"role":"user","parts":[{"text":"hello"}]}]}`
			w := performGeminiHandlerRequest(t, router, "deepseek-v4-pro", reqBody)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
			}
			tt.wantUpstream(t, captured)
			tt.wantDownstream(t, w.Body.Bytes())
		})
	}
}

func performGeminiStreamHandlerRequest(t *testing.T, router *gin.Engine, model string, body string) *httptest.ResponseRecorder {
	t.Helper()
	url := "/v1beta/models/" + model + ":streamGenerateContent"
	req := httptest.NewRequest(http.MethodPost, url, bytes.NewBufferString(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("x-api-key", "secret-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	return w
}

func TestGeminiHandler_DeepSeekChatAndMessagesStreamThinkingMatrix(t *testing.T) {
	tests := []struct {
		name         string
		serviceType  string
		responseBody string
		wantText     string
	}{
		{
			name:        "gemini_stream_to_deepseek_chat",
			serviceType: "openai",
			responseBody: strings.Join([]string{
				`data: {"id":"chatcmpl_ds","model":"deepseek-v4-pro","choices":[{"delta":{"reasoning_content":"chat reasoning"},"finish_reason":null}]}`,
				`data: {"id":"chatcmpl_ds","model":"deepseek-v4-pro","choices":[{"delta":{"content":"chat text"},"finish_reason":null}]}`,
				`data: {"id":"chatcmpl_ds","model":"deepseek-v4-pro","choices":[{"delta":{},"finish_reason":"stop"}]}`,
				`data: [DONE]`,
				``,
			}, "\n"),
			wantText: "chat reasoning",
		},
		{
			name:        "gemini_stream_to_deepseek_messages",
			serviceType: "claude",
			responseBody: strings.Join([]string{
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
				`event: message_delta`,
				`data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"input_tokens":1,"output_tokens":2}}`,
				``,
				`event: message_stop`,
				`data: {"type":"message_stop"}`,
				``,
			}, "\n"),
			wantText: "messages thinking",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "text/event-stream")
				w.WriteHeader(http.StatusOK)
				_, _ = w.Write([]byte(tt.responseBody))
			}))
			defer upstream.Close()

			router := newGeminiTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			})

			w := performGeminiStreamHandlerRequest(t, router, "deepseek-v4-pro", `{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}`)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
			}
			body := w.Body.String()
			if !strings.Contains(body, `"thought":true`) || !strings.Contains(body, tt.wantText) {
				t.Fatalf("expected Gemini thought stream part %q, got %s", tt.wantText, body)
			}
		})
	}
}
