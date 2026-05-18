package messages

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/gin-gonic/gin"
)

func setupMessagesTestConfigManager(t *testing.T, upstream []config.UpstreamConfig) *config.ConfigManager {
	t.Helper()
	cfg := config.Config{Upstream: upstream}
	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		t.Fatalf("serialize config: %v", err)
	}
	tmpFile := filepath.Join(t.TempDir(), "config.json")
	if err := os.WriteFile(tmpFile, data, 0644); err != nil {
		t.Fatalf("write config file: %v", err)
	}
	cm, err := config.NewConfigManager(tmpFile)
	if err != nil {
		t.Fatalf("NewConfigManager() err = %v", err)
	}
	t.Cleanup(func() { cm.Close() })
	return cm
}

func newMessagesTestRouter(t *testing.T, upstream config.UpstreamConfig) *gin.Engine {
	t.Helper()
	gin.SetMode(gin.TestMode)
	cfgManager := setupMessagesTestConfigManager(t, []config.UpstreamConfig{upstream})
	channelScheduler := scheduler.NewChannelScheduler(
		cfgManager,
		metrics.NewMetricsManager(),
		metrics.NewMetricsManager(),
		metrics.NewMetricsManager(),
		metrics.NewMetricsManager(),
		metrics.NewMetricsManager(),
		session.NewTraceAffinityManager(),
		nil,
	)
	envCfg := &config.EnvConfig{
		ProxyAccessKey:     "secret-key",
		MaxRequestBodySize: 1024 * 1024,
	}

	r := gin.New()
	r.POST("/v1/messages", Handler(envCfg, cfgManager, channelScheduler))
	return r
}

func performMessagesHandlerRequest(t *testing.T, router *gin.Engine, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/v1/messages", bytes.NewBufferString(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("x-api-key", "secret-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	return w
}

func TestMessagesHandler_NonStreamMatrix_AllFourUpstreams(t *testing.T) {
	tests := []struct {
		name              string
		serviceType       string
		responseBody      string
		expectedText      string
		expectedStop      string
		expectedInputTok  int
		expectedOutputTok int
	}{
		{
			name:              "messages_handler_to_claude",
			serviceType:       "claude",
			responseBody:      `{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{"input_tokens":11,"output_tokens":7}}`,
			expectedText:      "hi",
			expectedStop:      "end_turn",
			expectedInputTok:  11,
			expectedOutputTok: 7,
		},
		{
			name:              "messages_handler_to_openai",
			serviceType:       "openai",
			responseBody:      `{"id":"chatcmpl_1","choices":[{"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":13,"completion_tokens":5,"total_tokens":18}}`,
			expectedText:      "hi",
			expectedStop:      "end_turn",
			expectedInputTok:  13,
			expectedOutputTok: 5,
		},
		{
			name:              "messages_handler_to_gemini",
			serviceType:       "gemini",
			responseBody:      `{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":17,"candidatesTokenCount":3,"totalTokenCount":20}}`,
			expectedText:      "hi",
			expectedStop:      "end_turn",
			expectedInputTok:  17,
			expectedOutputTok: 3,
		},
		{
			name:              "messages_handler_to_responses",
			serviceType:       "responses",
			responseBody:      `{"id":"resp_1","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}],"usage":{"input_tokens":19,"output_tokens":9,"total_tokens":28}}`,
			expectedText:      "hi",
			expectedStop:      "end_turn",
			expectedInputTok:  19,
			expectedOutputTok: 9,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
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

			w := performMessagesHandlerRequest(t, router, `{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}`)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
			}

			var resp struct {
				ID         string `json:"id"`
				Type       string `json:"type"`
				Role       string `json:"role"`
				StopReason string `json:"stop_reason"`
				Content    []struct {
					Type string `json:"type"`
					Text string `json:"text"`
				} `json:"content"`
				Usage *struct {
					InputTokens  int `json:"input_tokens"`
					OutputTokens int `json:"output_tokens"`
				} `json:"usage"`
			}
			if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
				t.Fatalf("unmarshal response: %v, body=%s", err, w.Body.String())
			}

			if resp.StopReason != tt.expectedStop {
				t.Fatalf("stop_reason = %q, want %q", resp.StopReason, tt.expectedStop)
			}
			if len(resp.Content) == 0 || resp.Content[0].Type != "text" {
				t.Fatalf("content[0].type = %q, want text", func() string {
					if len(resp.Content) == 0 {
						return "<empty>"
					}
					return resp.Content[0].Type
				}())
			}
			if resp.Content[0].Text != tt.expectedText {
				t.Fatalf("content[0].text = %q, want %q", resp.Content[0].Text, tt.expectedText)
			}
			if resp.Usage == nil {
				t.Fatalf("usage is nil")
			}
			if resp.Usage.InputTokens != tt.expectedInputTok {
				t.Fatalf("input_tokens = %d, want %d", resp.Usage.InputTokens, tt.expectedInputTok)
			}
			if resp.Usage.OutputTokens != tt.expectedOutputTok {
				t.Fatalf("output_tokens = %d, want %d", resp.Usage.OutputTokens, tt.expectedOutputTok)
			}
		})
	}
}

func TestMessagesHandler_NonStreamMatrix_ToolUse(t *testing.T) {
	tests := []struct {
		name         string
		serviceType  string
		responseBody string
		expectTool   string
	}{
		{
			name:         "messages_handler_tool_from_openai",
			serviceType:  "openai",
			responseBody: `{"id":"chat_tool","choices":[{"message":{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"Read","arguments":"{\"file_path\":\"/tmp/x\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`,
			expectTool:   "Read",
		},
		{
			name:         "messages_handler_tool_from_gemini",
			serviceType:  "gemini",
			responseBody: `{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"search_docs","args":{"query":"go"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}`,
			expectTool:   "search_docs",
		},
		{
			name:         "messages_handler_tool_from_responses",
			serviceType:  "responses",
			responseBody: `{"id":"resp_tool","status":"completed","output":[{"type":"function_call","call_id":"call_r","name":"Read","arguments":"{\"file_path\":\"/tmp/y\"}"}],"usage":{"input_tokens":1,"output_tokens":1}}`,
			expectTool:   "Read",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
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

			w := performMessagesHandlerRequest(t, router, `{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":[{"type":"text","text":"read file"}]}]}`)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
			}

			var resp struct {
				Content []struct {
					Type string `json:"type"`
					Name string `json:"name"`
				} `json:"content"`
			}
			if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
				t.Fatalf("unmarshal: %v", err)
			}

			var found bool
			for _, c := range resp.Content {
				if c.Type == "tool_use" && c.Name == tt.expectTool {
					found = true
					break
				}
			}
			if !found {
				t.Fatalf("expected tool_use name=%q in response, got %#v", tt.expectTool, resp.Content)
			}
		})
	}
}

func TestMessagesHandler_PreservesValidThinkingBlocksForClaudeUpstream(t *testing.T) {
	var capturedBody []byte

	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Fatalf("read upstream request body: %v", err)
		}
		capturedBody = body

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}`))
	}))
	defer upstream.Close()

	router := newMessagesTestRouter(t, config.UpstreamConfig{
		Name:        "thinking-preserve",
		BaseURL:     upstream.URL,
		APIKeys:     []string{"sk-test"},
		ServiceType: "claude",
		Status:      "active",
	})

	reqBody := `{
		"model":"claude-sonnet-4-6",
		"messages":[
			{"role":"user","content":[{"type":"text","text":"hello"}]},
			{"role":"assistant","content":[
				{"type":"thinking","thinking":"internal","signature":"sig_123"},
				{"type":"text","text":"Let me check"}
			]}
		]
	}`

	w := performMessagesHandlerRequest(t, router, reqBody)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
	}
	if len(capturedBody) == 0 {
		t.Fatal("upstream request body is empty")
	}

	var upstreamReq map[string]interface{}
	if err := json.Unmarshal(capturedBody, &upstreamReq); err != nil {
		t.Fatalf("unmarshal upstream request: %v, body=%s", err, string(capturedBody))
	}

	messages, ok := upstreamReq["messages"].([]interface{})
	if !ok || len(messages) < 2 {
		t.Fatalf("unexpected messages in upstream request: %#v", upstreamReq["messages"])
	}
	assistantMsg, ok := messages[1].(map[string]interface{})
	if !ok {
		t.Fatalf("assistant message type = %T", messages[1])
	}
	content, ok := assistantMsg["content"].([]interface{})
	if !ok {
		t.Fatalf("assistant content type = %T", assistantMsg["content"])
	}

	var foundThinking bool
	for _, raw := range content {
		block, ok := raw.(map[string]interface{})
		if !ok {
			continue
		}
		if blockType, _ := block["type"].(string); blockType == "thinking" {
			foundThinking = true
			if thinking, _ := block["thinking"].(string); thinking != "internal" {
				t.Fatalf("thinking = %q, want %q", thinking, "internal")
			}
			if signature, _ := block["signature"].(string); signature != "sig_123" {
				t.Fatalf("signature = %q, want %q", signature, "sig_123")
			}
		}
	}
	if !foundThinking {
		t.Fatalf("thinking block not found in upstream request: %s", string(capturedBody))
	}
}
