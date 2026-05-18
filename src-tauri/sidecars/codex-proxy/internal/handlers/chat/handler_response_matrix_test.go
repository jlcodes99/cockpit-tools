package chat

import (
	"bytes"
	"encoding/json"
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

func setupChatTestConfigManager(t *testing.T, upstream []config.UpstreamConfig) *config.ConfigManager {
	t.Helper()
	cfg := config.Config{ChatUpstream: upstream}
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

func newChatTestRouter(t *testing.T, upstream config.UpstreamConfig) *gin.Engine {
	t.Helper()
	gin.SetMode(gin.TestMode)
	cfgManager := setupChatTestConfigManager(t, []config.UpstreamConfig{upstream})
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
	r.POST("/v1/chat/completions", Handler(envCfg, cfgManager, channelScheduler))
	return r
}

func performChatHandlerRequest(t *testing.T, router *gin.Engine, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", bytes.NewBufferString(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer secret-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	return w
}

// TestChatHandler_NonStreamMatrix_OpenAIFormat 验证 Chat 入口在 openai/claude 上游下
// 返回正确的 OpenAI Chat Completions 格式响应
func TestChatHandler_NonStreamMatrix_OpenAIFormat(t *testing.T) {
	tests := []struct {
		name                string
		serviceType         string
		responseBody        string
		expectedText        string
		expectedFinish      string
		expectedPromptTok   int
		expectedCompleteTok int
	}{
		{
			name:                "chat_handler_to_openai",
			serviceType:         "openai",
			responseBody:        `{"id":"chatcmpl_1","object":"chat.completion","model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":13,"completion_tokens":5,"total_tokens":18}}`,
			expectedText:        "hi",
			expectedFinish:      "stop",
			expectedPromptTok:   13,
			expectedCompleteTok: 5,
		},
		{
			name:                "chat_handler_to_claude",
			serviceType:         "claude",
			responseBody:        `{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{"input_tokens":11,"output_tokens":7}}`,
			expectedText:        "hi",
			expectedFinish:      "stop",
			expectedPromptTok:   11,
			expectedCompleteTok: 7,
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

			router := newChatTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			})

			w := performChatHandlerRequest(t, router, `{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}`)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
			}

			var resp struct {
				ID      string `json:"id"`
				Object  string `json:"object"`
				Choices []struct {
					Index   int `json:"index"`
					Message struct {
						Role    string `json:"role"`
						Content string `json:"content"`
					} `json:"message"`
					FinishReason string `json:"finish_reason"`
				} `json:"choices"`
				Usage *struct {
					PromptTokens     int `json:"prompt_tokens"`
					CompletionTokens int `json:"completion_tokens"`
					TotalTokens      int `json:"total_tokens"`
				} `json:"usage"`
			}
			if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
				t.Fatalf("unmarshal response: %v, body=%s", err, w.Body.String())
			}

			if len(resp.Choices) == 0 {
				t.Fatalf("choices empty: %s", w.Body.String())
			}
			if resp.Choices[0].Message.Content != tt.expectedText {
				t.Fatalf("content = %q, want %q", resp.Choices[0].Message.Content, tt.expectedText)
			}
			if resp.Choices[0].FinishReason != tt.expectedFinish {
				t.Fatalf("finish_reason = %q, want %q", resp.Choices[0].FinishReason, tt.expectedFinish)
			}
			if resp.Usage == nil {
				t.Fatalf("usage is nil")
			}
			if resp.Usage.PromptTokens != tt.expectedPromptTok {
				t.Fatalf("prompt_tokens = %d, want %d", resp.Usage.PromptTokens, tt.expectedPromptTok)
			}
			if resp.Usage.CompletionTokens != tt.expectedCompleteTok {
				t.Fatalf("completion_tokens = %d, want %d", resp.Usage.CompletionTokens, tt.expectedCompleteTok)
			}
		})
	}
}

// TestChatHandler_NonStreamMatrix_Passthrough 验证 Chat 入口对 gemini/responses 上游
// 走透传路径：上游响应原样返回给客户端（不做格式转换）
func TestChatHandler_NonStreamMatrix_Passthrough(t *testing.T) {
	tests := []struct {
		name         string
		serviceType  string
		responseBody string
	}{
		{
			name:         "chat_handler_to_gemini_passthrough",
			serviceType:  "gemini",
			responseBody: `{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":17,"candidatesTokenCount":3,"totalTokenCount":20}}`,
		},
		{
			name:         "chat_handler_to_responses_passthrough",
			serviceType:  "responses",
			responseBody: `{"id":"resp_1","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}],"usage":{"input_tokens":19,"output_tokens":9,"total_tokens":28}}`,
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

			router := newChatTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			})

			w := performChatHandlerRequest(t, router, `{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}`)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
			}

			// 透传路径：响应体应与上游原始输出一致
			if !bytes.Contains(w.Body.Bytes(), []byte(tt.responseBody[:50])) {
				t.Fatalf("response body does not contain upstream output prefix, got %s", w.Body.String())
			}
		})
	}
}

func TestChatHandler_PassthroughPreservesMultimodalRequest(t *testing.T) {
	tests := []struct {
		name        string
		serviceType string
	}{
		{name: "handler_openai_multimodal_passthrough", serviceType: "openai"},
		{name: "handler_responses_multimodal_passthrough", serviceType: "responses"},
		{name: "handler_gemini_multimodal_passthrough", serviceType: "gemini"},
	}

	requestBody := `{"model":"gpt-4o-image","messages":[{"role":"user","content":[{"type":"text","text":"修改这个图片"},{"type":"image_url","image_url":{"url":"https://example.com/image.png"}}]}]}`

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var captured map[string]interface{}
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				defer r.Body.Close()
				if err := json.NewDecoder(r.Body).Decode(&captured); err != nil {
					t.Fatalf("decode upstream request: %v", err)
				}
				w.Header().Set("Content-Type", "application/json")
				w.WriteHeader(http.StatusOK)
				_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","model":"gpt-4o-image","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`))
			}))
			defer upstream.Close()

			router := newChatTestRouter(t, config.UpstreamConfig{
				Name:        tt.name,
				BaseURL:     upstream.URL,
				APIKeys:     []string{"sk-test"},
				ServiceType: tt.serviceType,
				Status:      "active",
			})

			w := performChatHandlerRequest(t, router, requestBody)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
			}

			messages, ok := captured["messages"].([]interface{})
			if !ok || len(messages) != 1 {
				t.Fatalf("captured messages = %#v, want single message", captured["messages"])
			}

			message, ok := messages[0].(map[string]interface{})
			if !ok {
				t.Fatalf("captured message = %#v, want object", messages[0])
			}

			content, ok := message["content"].([]interface{})
			if !ok || len(content) != 2 {
				t.Fatalf("captured content = %#v, want 2-part array", message["content"])
			}

			imagePart, ok := content[1].(map[string]interface{})
			if !ok || imagePart["type"] != "image_url" {
				t.Fatalf("captured image part = %#v, want image_url", content[1])
			}

			imageURL, ok := imagePart["image_url"].(map[string]interface{})
			if !ok || imageURL["url"] != "https://example.com/image.png" {
				t.Fatalf("captured image_url = %#v, want original url", imagePart["image_url"])
			}
		})
	}
}

func TestChatHandler_ImageEditPassthroughSucceeds(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer r.Body.Close()

		var captured map[string]interface{}
		if err := json.NewDecoder(r.Body).Decode(&captured); err != nil {
			t.Fatalf("decode upstream request: %v", err)
		}

		messages, ok := captured["messages"].([]interface{})
		if !ok || len(messages) != 1 {
			t.Fatalf("captured messages = %#v, want single message", captured["messages"])
		}

		message, ok := messages[0].(map[string]interface{})
		if !ok {
			t.Fatalf("captured message = %#v, want object", messages[0])
		}

		content, ok := message["content"].([]interface{})
		if !ok || len(content) != 2 {
			t.Fatalf("captured content = %#v, want 2-part array", message["content"])
		}

		textPart, ok := content[0].(map[string]interface{})
		if !ok || textPart["type"] != "text" || textPart["text"] != "修改这个图片" {
			t.Fatalf("captured text part = %#v, want original prompt", content[0])
		}

		imagePart, ok := content[1].(map[string]interface{})
		if !ok || imagePart["type"] != "image_url" {
			t.Fatalf("captured image part = %#v, want image_url", content[1])
		}

		imageURL, ok := imagePart["image_url"].(map[string]interface{})
		if !ok || imageURL["url"] != "https://example.com/image.png" {
			t.Fatalf("captured image_url = %#v, want original url", imagePart["image_url"])
		}

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"id":"chatcmpl_imgedit_1","object":"chat.completion","created":1677652288,"choices":[{"index":0,"message":{"role":"assistant","content":"图片已修改完成"},"finish_reason":"stop"}],"usage":{"prompt_tokens":9,"completion_tokens":12,"total_tokens":21}}`))
	}))
	defer upstream.Close()

	router := newChatTestRouter(t, config.UpstreamConfig{
		Name:        "chat_openai_image_edit",
		BaseURL:     upstream.URL,
		APIKeys:     []string{"sk-test"},
		ServiceType: "openai",
		Status:      "active",
	})

	requestBody := `{"model":"gpt-4o-image","stream":false,"messages":[{"role":"user","content":[{"type":"text","text":"修改这个图片"},{"type":"image_url","image_url":{"url":"https://example.com/image.png"}}]}]}`
	w := performChatHandlerRequest(t, router, requestBody)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
	}

	var resp struct {
		ID      string `json:"id"`
		Object  string `json:"object"`
		Choices []struct {
			Message struct {
				Role    string `json:"role"`
				Content string `json:"content"`
			} `json:"message"`
			FinishReason string `json:"finish_reason"`
		} `json:"choices"`
		Usage struct {
			PromptTokens     int `json:"prompt_tokens"`
			CompletionTokens int `json:"completion_tokens"`
			TotalTokens      int `json:"total_tokens"`
		} `json:"usage"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal response: %v, body=%s", err, w.Body.String())
	}

	if resp.ID != "chatcmpl_imgedit_1" {
		t.Fatalf("id = %q, want chatcmpl_imgedit_1", resp.ID)
	}
	if resp.Object != "chat.completion" {
		t.Fatalf("object = %q, want chat.completion", resp.Object)
	}
	if len(resp.Choices) != 1 {
		t.Fatalf("choices = %#v, want single choice", resp.Choices)
	}
	if resp.Choices[0].Message.Content != "图片已修改完成" {
		t.Fatalf("content = %q, want 图片已修改完成", resp.Choices[0].Message.Content)
	}
	if resp.Choices[0].FinishReason != "stop" {
		t.Fatalf("finish_reason = %q, want stop", resp.Choices[0].FinishReason)
	}
	if resp.Usage.PromptTokens != 9 || resp.Usage.CompletionTokens != 12 || resp.Usage.TotalTokens != 21 {
		t.Fatalf("usage = %#v, want {9 12 21}", resp.Usage)
	}
}

func TestChatHandler_NonStreamMatrix_ToolCalls(t *testing.T) {
	// Claude 上游走 convertClaudeResponseToChat 转换，能正确输出 OpenAI tool_calls 格式
	t.Run("chat_handler_tool_from_claude", func(t *testing.T) {
		upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusOK)
			_, _ = w.Write([]byte(`{"id":"msg_tool","type":"message","role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"Read","input":{"file_path":"/tmp/a"}}],"stop_reason":"tool_use","usage":{"input_tokens":1,"output_tokens":1}}`))
		}))
		defer upstream.Close()

		router := newChatTestRouter(t, config.UpstreamConfig{
			Name:        "chat_claude_tool",
			BaseURL:     upstream.URL,
			APIKeys:     []string{"sk-test"},
			ServiceType: "claude",
			Status:      "active",
		})

		w := performChatHandlerRequest(t, router, `{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":"read file"}]}`)
		if w.Code != http.StatusOK {
			t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
		}

		var resp struct {
			Choices []struct {
				Message struct {
					ToolCalls []struct {
						Function struct {
							Name string `json:"name"`
						} `json:"function"`
					} `json:"tool_calls"`
				} `json:"message"`
				FinishReason string `json:"finish_reason"`
			} `json:"choices"`
		}
		if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
			t.Fatalf("unmarshal: %v", err)
		}
		if len(resp.Choices) == 0 || len(resp.Choices[0].Message.ToolCalls) == 0 {
			t.Fatalf("expected tool_calls in response, got %s", w.Body.String())
		}
		if resp.Choices[0].Message.ToolCalls[0].Function.Name != "Read" {
			t.Fatalf("tool name = %q, want Read", resp.Choices[0].Message.ToolCalls[0].Function.Name)
		}
		if resp.Choices[0].FinishReason != "tool_calls" {
			t.Fatalf("finish_reason = %q, want tool_calls", resp.Choices[0].FinishReason)
		}
	})

	// openai 上游透传，上游已是 OpenAI tool_calls 格式
	t.Run("chat_handler_tool_from_openai", func(t *testing.T) {
		upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusOK)
			_, _ = w.Write([]byte(`{"id":"chat_tool","model":"gpt-4o","choices":[{"message":{"role":"assistant","tool_calls":[{"id":"call_o","type":"function","function":{"name":"Read","arguments":"{\"file_path\":\"/tmp/b\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`))
		}))
		defer upstream.Close()

		router := newChatTestRouter(t, config.UpstreamConfig{
			Name:        "chat_openai_tool",
			BaseURL:     upstream.URL,
			APIKeys:     []string{"sk-test"},
			ServiceType: "openai",
			Status:      "active",
		})

		w := performChatHandlerRequest(t, router, `{"model":"gpt-4o","messages":[{"role":"user","content":"read file"}]}`)
		if w.Code != http.StatusOK {
			t.Fatalf("status = %d, want %d, body=%s", w.Code, http.StatusOK, w.Body.String())
		}

		var resp struct {
			Choices []struct {
				Message struct {
					ToolCalls []struct {
						Function struct {
							Name string `json:"name"`
						} `json:"function"`
					} `json:"tool_calls"`
				} `json:"message"`
			} `json:"choices"`
		}
		if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
			t.Fatalf("unmarshal: %v", err)
		}
		if len(resp.Choices) == 0 || len(resp.Choices[0].Message.ToolCalls) == 0 {
			t.Fatalf("expected tool_calls in response, got %s", w.Body.String())
		}
		if resp.Choices[0].Message.ToolCalls[0].Function.Name != "Read" {
			t.Fatalf("tool name = %q, want Read", resp.Choices[0].Message.ToolCalls[0].Function.Name)
		}
	})
}
