package providers

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

func TestResponsesEntry_RequestMatrix_AllFourUpstreams(t *testing.T) {
	gin.SetMode(gin.TestMode)
	tests := []struct {
		name            string
		serviceType     string
		path            string
		body            string
		expectedURL     string
		expectedModel   string
		expectFieldPath string
	}{
		{
			name:            "responses_to_responses",
			serviceType:     "responses",
			path:            "/v1/responses",
			body:            `{"model":"gpt-5","input":"hello","stream":true}`,
			expectedURL:     "https://api.example.com/v1/responses",
			expectedModel:   "gpt-5.2",
			expectFieldPath: "input",
		},
		{
			name:            "responses_to_claude",
			serviceType:     "claude",
			path:            "/v1/responses",
			body:            `{"model":"gpt-5","input":"hello","stream":false}`,
			expectedURL:     "https://api.example.com/v1/messages",
			expectedModel:   "claude-3-5-sonnet",
			expectFieldPath: "messages",
		},
		{
			name:            "responses_to_openai",
			serviceType:     "openai",
			path:            "/v1/responses",
			body:            `{"model":"gpt-5","input":"hello","stream":false}`,
			expectedURL:     "https://api.example.com/v1/chat/completions",
			expectedModel:   "gpt-5.2",
			expectFieldPath: "messages",
		},
		{
			name:            "responses_to_gemini",
			serviceType:     "gemini",
			path:            "/v1/responses",
			body:            `{"model":"gpt-5","input":"hello","stream":false}`,
			expectedURL:     "https://api.example.com/v1beta/models/gpt-5.2:generateContent",
			expectedModel:   "gpt-5.2",
			expectFieldPath: "contents",
		},
		{
			name:            "responses_hash_baseurl_openai",
			serviceType:     "openai",
			path:            "/v1/responses",
			body:            `{"model":"gpt-5","input":"hello"}`,
			expectedURL:     "https://core.blink.new/api/v1/ai/chat/completions",
			expectedModel:   "gpt-5.2",
			expectFieldPath: "messages",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := newGinContext(http.MethodPost, tt.path, []byte(tt.body), context.Background())
			upstream := &config.UpstreamConfig{
				BaseURL:      "https://api.example.com",
				ServiceType:  tt.serviceType,
				ModelMapping: map[string]string{"gpt-5": tt.expectedModel},
			}
			if tt.name == "responses_hash_baseurl_openai" {
				upstream.BaseURL = "https://core.blink.new/api/v1/ai#"
			}

			provider := &ResponsesProvider{}
			req, _, err := provider.ConvertToProviderRequest(c, upstream, "sk-test")
			if err != nil {
				t.Fatalf("ConvertToProviderRequest() err = %v", err)
			}
			if req.URL.String() != tt.expectedURL {
				t.Fatalf("url = %s, want %s", req.URL.String(), tt.expectedURL)
			}

			var body map[string]interface{}
			if err := json.NewDecoder(req.Body).Decode(&body); err != nil {
				t.Fatalf("decode body: %v", err)
			}
			if _, ok := body[tt.expectFieldPath]; !ok {
				t.Fatalf("expected field %q in request body, got %#v", tt.expectFieldPath, body)
			}
		})
	}
}

func TestResponsesEntry_RequestMatrix_PreservesKeyParams(t *testing.T) {
	gin.SetMode(gin.TestMode)

	t.Run("responses_passthrough_preserves_instructions_and_stream", func(t *testing.T) {
		c := newGinContext(http.MethodPost, "/v1/responses", []byte(`{"model":"gpt-5","input":"hi","instructions":"be helpful","stream":true,"max_output_tokens":1024}`), context.Background())
		upstream := &config.UpstreamConfig{
			BaseURL:     "https://api.example.com",
			ServiceType: "responses",
		}

		provider := &ResponsesProvider{}
		req, _, err := provider.ConvertToProviderRequest(c, upstream, "sk-test")
		if err != nil {
			t.Fatalf("ConvertToProviderRequest() err = %v", err)
		}

		var body map[string]interface{}
		if err := json.NewDecoder(req.Body).Decode(&body); err != nil {
			t.Fatalf("decode body: %v", err)
		}

		if body["instructions"] != "be helpful" {
			t.Fatalf("instructions = %v, want be helpful", body["instructions"])
		}
		if body["stream"] != true {
			t.Fatalf("stream = %v, want true", body["stream"])
		}
		// passthrough 模式下 max_output_tokens 保留原字段名
		if maxTok, ok := body["max_output_tokens"]; !ok || maxTok == nil {
			t.Fatalf("max_output_tokens field missing in passthrough body, got %#v", body)
		}
	})

	t.Run("responses_to_claude_preserves_instructions_as_system", func(t *testing.T) {
		c := newGinContext(http.MethodPost, "/v1/responses", []byte(`{"model":"claude","input":"hi","instructions":"be helpful","stream":false}`), context.Background())
		upstream := &config.UpstreamConfig{
			BaseURL:     "https://api.example.com",
			ServiceType: "claude",
		}

		provider := &ResponsesProvider{}
		req, _, err := provider.ConvertToProviderRequest(c, upstream, "sk-test")
		if err != nil {
			t.Fatalf("ConvertToProviderRequest() err = %v", err)
		}

		var body map[string]interface{}
		if err := json.NewDecoder(req.Body).Decode(&body); err != nil {
			t.Fatalf("decode body: %v", err)
		}

		// Claude 上游应该将 instructions 转为 system 字段
		if system, ok := body["system"]; !ok || system == nil {
			t.Fatalf("system field missing in Claude request body, got %#v", body)
		}
	})

	t.Run("responses_to_gemini_uses_model_in_url", func(t *testing.T) {
		c := newGinContext(http.MethodPost, "/v1/responses", []byte(`{"model":"gemini-pro","input":"hi","stream":false}`), context.Background())
		upstream := &config.UpstreamConfig{
			BaseURL:      "https://api.example.com",
			ServiceType:  "gemini",
			ModelMapping: map[string]string{"gemini-pro": "gemini-2.5-pro"},
		}

		provider := &ResponsesProvider{}
		req, _, err := provider.ConvertToProviderRequest(c, upstream, "sk-test")
		if err != nil {
			t.Fatalf("ConvertToProviderRequest() err = %v", err)
		}

		expectedURL := "https://api.example.com/v1beta/models/gemini-2.5-pro:generateContent"
		if req.URL.String() != expectedURL {
			t.Fatalf("url = %s, want %s", req.URL.String(), expectedURL)
		}
	})
}

func TestResponsesProvider_NormalizeNonstandardChatRolesForOpenAIChatUpstream(t *testing.T) {
	gin.SetMode(gin.TestMode)

	tests := []struct {
		name      string
		enabled   bool
		wantFirst string
	}{
		{name: "default_off", enabled: false, wantFirst: "developer"},
		{name: "enabled", enabled: true, wantFirst: "user"},
	}

	body := []byte(`{"model":"gpt-5","input":[{"type":"message","role":"developer","content":"dev message"},{"type":"message","role":"user","content":"user message"}]}`)
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := newGinContext(http.MethodPost, "/v1/responses", body, context.Background())
			upstream := &config.UpstreamConfig{
				BaseURL:                       "https://api.example.com",
				ServiceType:                   "openai",
				NormalizeNonstandardChatRoles: tt.enabled,
			}

			provider := &ResponsesProvider{}
			req, _, err := provider.ConvertToProviderRequest(c, upstream, "sk-test")
			if err != nil {
				t.Fatalf("ConvertToProviderRequest() err = %v", err)
			}

			var got map[string]interface{}
			if err := json.NewDecoder(req.Body).Decode(&got); err != nil {
				t.Fatalf("decode body: %v", err)
			}

			messages, ok := got["messages"].([]interface{})
			if !ok || len(messages) != 2 {
				t.Fatalf("messages = %#v, want 2 items", got["messages"])
			}
			first, ok := messages[0].(map[string]interface{})
			if !ok {
				t.Fatalf("message[0] = %#v, want object", messages[0])
			}
			if first["role"] != tt.wantFirst {
				t.Fatalf("message[0].role = %v, want %s", first["role"], tt.wantFirst)
			}
		})
	}
}
