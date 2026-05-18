package chat

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

func TestChatEntry_RequestMatrix_AllFourUpstreams(t *testing.T) {
	gin.SetMode(gin.TestMode)
	tests := []struct {
		name            string
		serviceType     string
		expectedURL     string
		expectFieldPath string
	}{
		{"chat_to_openai", "openai", "https://api.example.com/v1/chat/completions", "messages"},
		{"chat_to_claude", "claude", "https://api.example.com/v1/messages", "messages"},
		{"chat_to_gemini", "gemini", "https://api.example.com/v1/chat/completions", "messages"},
		{"chat_to_responses", "responses", "https://api.example.com/v1/chat/completions", "messages"},
		{"chat_hash_baseurl", "openai", "https://core.blink.new/api/v1/ai/chat/completions", "messages"},
	}

	bodyBytes := []byte(`{"model":"gpt-5","messages":[{"role":"user","content":"hi"}]}`)
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil).WithContext(context.Background())

			upstream := &config.UpstreamConfig{BaseURL: "https://api.example.com", ServiceType: tt.serviceType}
			if tt.name == "chat_hash_baseurl" {
				upstream.BaseURL = "https://core.blink.new/api/v1/ai#"
			}
			req, err := buildProviderRequest(c, upstream, upstream.BaseURL, "sk-test", bodyBytes, "gpt-5", false)
			if err != nil {
				t.Fatalf("buildProviderRequest() err = %v", err)
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
