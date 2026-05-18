package responses

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/gin-gonic/gin"
)

func setupResponsesTestConfigManager(t *testing.T, upstream []config.UpstreamConfig) *config.ConfigManager {
	t.Helper()
	cfg := config.Config{ResponsesUpstream: upstream}
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

func newResponsesTestRouter(t *testing.T, upstream config.UpstreamConfig, sessionManager *session.SessionManager) *gin.Engine {
	t.Helper()
	gin.SetMode(gin.TestMode)
	cfgManager := setupResponsesTestConfigManager(t, []config.UpstreamConfig{upstream})
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
	r.POST("/v1/responses", Handler(envCfg, cfgManager, sessionManager, channelScheduler))
	return r
}

func performResponsesHandlerRequest(t *testing.T, router *gin.Engine, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/v1/responses", bytes.NewBufferString(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("x-api-key", "secret-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	return w
}

func TestResponsesHandler_NonStreamMatrix_AllFourUpstreams(t *testing.T) {
	sessionManager := session.NewSessionManager(time.Hour, 100, 100000)

	tests := []struct {
		name              string
		serviceType       string
		responseBody      string
		expectedText      string
		expectedStatus    string
		expectedInputTok  int
		expectedOutputTok int
	}{
		{
			name:              "responses_handler_to_responses",
			serviceType:       "responses",
			responseBody:      `{"id":"resp_native","model":"gpt-5","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}],"usage":{"input_tokens":11,"output_tokens":7,"total_tokens":18}}`,
			expectedText:      "hi",
			expectedStatus:    "completed",
			expectedInputTok:  11,
			expectedOutputTok: 7,
		},
		{
			name:              "responses_handler_to_claude",
			serviceType:       "claude",
			responseBody:      `{"id":"msg_1","model":"claude-3-5-sonnet","content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{"input_tokens":13,"output_tokens":5}}`,
			expectedText:      "hi",
			expectedStatus:    "completed",
			expectedInputTok:  13,
			expectedOutputTok: 5,
		},
		{
			name:              "responses_handler_to_openai",
			serviceType:       "openai",
			responseBody:      `{"id":"chatcmpl_1","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":17,"completion_tokens":3,"total_tokens":20}}`,
			expectedText:      "hi",
			expectedStatus:    "completed",
			expectedInputTok:  17,
			expectedOutputTok: 3,
		},
		{
			name:              "responses_handler_to_gemini",
			serviceType:       "gemini",
			responseBody:      `{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":19,"candidatesTokenCount":9,"totalTokenCount":28}}`,
			expectedText:      "hi",
			expectedStatus:    "completed",
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

			router := newResponsesTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			}, sessionManager)

			w := performResponsesHandlerRequest(t, router, `{"model":"gpt-5","input":"hello"}`)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
			}

			var resp struct {
				ID     string `json:"id"`
				Status string `json:"status"`
				Output []struct {
					Type    string      `json:"type"`
					Role    string      `json:"role"`
					Content interface{} `json:"content"`
				} `json:"output"`
				Usage struct {
					InputTokens  int `json:"input_tokens"`
					OutputTokens int `json:"output_tokens"`
				} `json:"usage"`
			}
			if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
				t.Fatalf("unmarshal response: %v, body=%s", err, w.Body.String())
			}

			if resp.Status != tt.expectedStatus {
				t.Fatalf("status = %q, want %q", resp.Status, tt.expectedStatus)
			}
			if len(resp.Output) == 0 {
				t.Fatalf("output empty: %s", w.Body.String())
			}
			if got := fmt.Sprint(resp.Output[0].Content); got == "<nil>" {
				t.Fatalf("first output content is nil: %#v", resp.Output)
			}
			if resp.Usage.InputTokens != tt.expectedInputTok {
				t.Fatalf("input_tokens = %d, want %d", resp.Usage.InputTokens, tt.expectedInputTok)
			}
			if resp.Usage.OutputTokens != tt.expectedOutputTok {
				t.Fatalf("output_tokens = %d, want %d", resp.Usage.OutputTokens, tt.expectedOutputTok)
			}

			bodyText := w.Body.String()
			if !bytes.Contains([]byte(bodyText), []byte(tt.expectedText)) {
				t.Fatalf("response body %q does not contain expected text %q", bodyText, tt.expectedText)
			}
		})
	}
}

func TestResponsesHandler_NonStreamMatrix_FunctionCall(t *testing.T) {
	sessionManager := session.NewSessionManager(time.Hour, 100, 100000)

	tests := []struct {
		name         string
		serviceType  string
		responseBody string
		expectCall   string
		expectName   string
	}{
		{
			name:         "responses_handler_function_call_from_claude",
			serviceType:  "claude",
			responseBody: `{"id":"msg_tool","model":"claude-3-5-sonnet","content":[{"type":"tool_use","id":"call_claude","name":"Read","input":{"file_path":"/tmp/a"}}],"stop_reason":"tool_use","usage":{"input_tokens":1,"output_tokens":1}}`,
			expectCall:   "call_claude",
			expectName:   "Read",
		},
		{
			name:         "responses_handler_function_call_from_openai",
			serviceType:  "openai",
			responseBody: `{"id":"chat_tool","model":"gpt-4o","choices":[{"message":{"role":"assistant","tool_calls":[{"id":"call_openai","type":"function","function":{"name":"Read","arguments":"{\"file_path\":\"/tmp/b\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`,
			expectCall:   "call_openai",
			expectName:   "Read",
		},
		{
			name:         "responses_handler_function_call_from_gemini",
			serviceType:  "gemini",
			responseBody: `{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"search_docs","args":{"query":"responses"}}}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}`,
			expectCall:   "search_docs",
			expectName:   "search_docs",
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

			router := newResponsesTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			}, sessionManager)

			w := performResponsesHandlerRequest(t, router, `{"model":"gpt-5","input":"hello"}`)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
			}
			if !bytes.Contains(w.Body.Bytes(), []byte(`"type":"function_call"`)) {
				t.Fatalf("expected function_call in response body, got %s", w.Body.String())
			}
			if !bytes.Contains(w.Body.Bytes(), []byte(tt.expectCall)) {
				t.Fatalf("expected call id %q in response body, got %s", tt.expectCall, w.Body.String())
			}
			if !bytes.Contains(w.Body.Bytes(), []byte(tt.expectName)) {
				t.Fatalf("expected tool/function name %q in response body, got %s", tt.expectName, w.Body.String())
			}
		})
	}
}
