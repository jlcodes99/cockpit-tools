package gemini

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/gin-gonic/gin"
)

func TestGeminiEntry_RequestMatrix_AllFourUpstreams(t *testing.T) {
	gin.SetMode(gin.TestMode)
	geminiReq := &types.GeminiRequest{
		Contents: []types.GeminiContent{{
			Role:  "user",
			Parts: []types.GeminiPart{{Text: "hi"}},
		}},
	}

	tests := []struct {
		name            string
		serviceType     string
		expectedURL     string
		expectFieldPath string
	}{
		{"gemini_to_gemini", "gemini", "https://api.example.com/v1beta/models/gemini-2.0-flash:generateContent", "contents"},
		{"gemini_to_claude", "claude", "https://api.example.com/v1/messages", "messages"},
		{"gemini_to_openai", "openai", "https://api.example.com/v1/chat/completions", "messages"},
		{"gemini_to_responses", "responses", "https://api.example.com/v1/responses", "input"},
		{"gemini_hash_baseurl_openai", "openai", "https://core.blink.new/api/v1/ai/v1/chat/completions", "messages"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest(http.MethodPost, "/v1beta/models/gemini-2.0-flash:generateContent", nil).WithContext(context.Background())

			upstream := &config.UpstreamConfig{BaseURL: "https://api.example.com", ServiceType: tt.serviceType}
			if tt.name == "gemini_hash_baseurl_openai" {
				upstream.BaseURL = "https://core.blink.new/api/v1/ai#"
			}
			req, err := buildProviderRequest(c, upstream, upstream.BaseURL, "test-key", geminiReq, "gemini-2.0-flash", false)
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
