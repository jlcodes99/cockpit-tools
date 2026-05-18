package handlers

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

func TestUnifiedSessionIDAffinityConsistencyAcrossAPIs(t *testing.T) {
	gin.SetMode(gin.TestMode)

	tests := []struct {
		name     string
		path     string
		headers  map[string]string
		body     string
		expected string
	}{
		{
			name: "same Session_id works for messages",
			path: "/v1/messages",
			headers: map[string]string{
				"Session_id": "shared-session-id",
			},
			body:     `{"model":"claude-opus-4-6","messages":[{"role":"user","content":"hello"}],"metadata":{"user_id":"meta-user"}}`,
			expected: "shared-session-id",
		},
		{
			name: "same Session_id works for responses",
			path: "/v1/responses",
			headers: map[string]string{
				"Session_id": "shared-session-id",
			},
			body:     `{"model":"gpt-5","input":"hello","prompt_cache_key":"body-cache-key","metadata":{"user_id":"meta-user"}}`,
			expected: "shared-session-id",
		},
		{
			name: "same Session_id works for chat",
			path: "/v1/chat/completions",
			headers: map[string]string{
				"Session_id": "shared-session-id",
			},
			body:     `{"model":"gpt-4.1","messages":[{"role":"user","content":"hello"}],"user":"chat-user"}`,
			expected: "shared-session-id",
		},
		{
			name: "same Session_id works for gemini",
			path: "/v1beta/models/gemini-2.0-flash:generateContent",
			headers: map[string]string{
				"Session_id": "shared-session-id",
			},
			body:     `{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}`,
			expected: "shared-session-id",
		},
		{
			name: "same Claude Code header works across messages",
			path: "/v1/messages",
			headers: map[string]string{
				"X-Claude-Code-Session-Id": "claude-code-session",
			},
			body:     `{"model":"claude-opus-4-6","messages":[{"role":"user","content":"hello"}],"metadata":{"user_id":"meta-user"}}`,
			expected: "claude-code-session",
		},
		{
			name: "same Claude Code header works across responses",
			path: "/v1/responses",
			headers: map[string]string{
				"X-Claude-Code-Session-Id": "claude-code-session",
			},
			body:     `{"model":"gpt-5","input":"hello","prompt_cache_key":"body-cache-key","metadata":{"user_id":"meta-user"}}`,
			expected: "claude-code-session",
		},
		{
			name: "same Claude Code header works across chat",
			path: "/v1/chat/completions",
			headers: map[string]string{
				"X-Claude-Code-Session-Id": "claude-code-session",
			},
			body:     `{"model":"gpt-4.1","messages":[{"role":"user","content":"hello"}],"user":"chat-user"}`,
			expected: "claude-code-session",
		},
		{
			name: "same Claude Code header works across gemini",
			path: "/v1beta/models/gemini-2.0-flash:generateContent",
			headers: map[string]string{
				"X-Claude-Code-Session-Id": "claude-code-session",
			},
			body:     `{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}`,
			expected: "claude-code-session",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest(http.MethodPost, tt.path, bytes.NewBufferString(tt.body))
			for k, v := range tt.headers {
				c.Request.Header.Set(k, v)
			}

			got := utils.ExtractUnifiedSessionID(c, []byte(tt.body))
			if got != tt.expected {
				t.Fatalf("ExtractUnifiedSessionID(%s) = %q, want %q", tt.path, got, tt.expected)
			}
		})
	}
}

func TestUnifiedSessionIDBodyFallbackConsistencyAcrossAPIs(t *testing.T) {
	gin.SetMode(gin.TestMode)

	tests := []struct {
		name     string
		path     string
		body     string
		expected string
	}{
		{
			name:     "messages falls back to metadata user id",
			path:     "/v1/messages",
			body:     `{"model":"claude-opus-4-6","messages":[{"role":"user","content":"hello"}],"metadata":{"user_id":"meta-fallback"}}`,
			expected: "meta-fallback",
		},
		{
			name:     "responses falls back to prompt cache key before metadata user id",
			path:     "/v1/responses",
			body:     `{"model":"gpt-5","input":"hello","prompt_cache_key":"responses-cache-key","metadata":{"user_id":"meta-fallback"}}`,
			expected: "responses-cache-key",
		},
		{
			name:     "chat falls back to user field",
			path:     "/v1/chat/completions",
			body:     `{"model":"gpt-4.1","messages":[{"role":"user","content":"hello"}],"user":"chat-fallback-user"}`,
			expected: "chat-fallback-user",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest(http.MethodPost, tt.path, bytes.NewBufferString(tt.body))

			got := utils.ExtractUnifiedSessionID(c, []byte(tt.body))
			if got != tt.expected {
				t.Fatalf("ExtractUnifiedSessionID(%s) = %q, want %q", tt.path, got, tt.expected)
			}
		})
	}
}
