package common

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gin-gonic/gin"
)

func newTestContext() *gin.Context {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request, _ = http.NewRequest("POST", "/", nil)
	return c
}

func TestHasImageContent_ClaudeMessages(t *testing.T) {
	tests := []struct {
		name     string
		body     string
		expected bool
	}{
		{
			name:     "claude image block base64",
			body:     `{"messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64","data":"abc"}}]}]}`,
			expected: true,
		},
		{
			name:     "claude image block url",
			body:     `{"messages":[{"role":"user","content":[{"type":"image","source":{"type":"url","url":"https://example.com/img.png"}}]}]}`,
			expected: true,
		},
		{
			name:     "claude text only",
			body:     `{"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}`,
			expected: false,
		},
		{
			name:     "claude string content",
			body:     `{"messages":[{"role":"user","content":"hello"}]}`,
			expected: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := newTestContext()
			got := HasImageContent(c, []byte(tt.body))
			if got != tt.expected {
				t.Errorf("HasImageContent() = %v, want %v", got, tt.expected)
			}
		})
	}
}

func TestHasImageContent_OpenAIChat(t *testing.T) {
	tests := []struct {
		name     string
		body     string
		expected bool
	}{
		{
			name:     "openai image_url block",
			body:     `{"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.com/img.png"}}]}]}`,
			expected: true,
		},
		{
			name:     "openai text only",
			body:     `{"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}`,
			expected: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := newTestContext()
			got := HasImageContent(c, []byte(tt.body))
			if got != tt.expected {
				t.Errorf("HasImageContent() = %v, want %v", got, tt.expected)
			}
		})
	}
}

func TestHasImageContent_Responses(t *testing.T) {
	tests := []struct {
		name     string
		body     string
		expected bool
	}{
		{
			name:     "responses input_image top level",
			body:     `{"input":[{"type":"input_image","image_url":"https://example.com/img.png"}]}`,
			expected: true,
		},
		{
			name:     "responses input_image nested in content",
			body:     `{"input":[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"https://example.com/img.png"}]}]}`,
			expected: true,
		},
		{
			name:     "responses text only",
			body:     `{"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]}`,
			expected: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := newTestContext()
			got := HasImageContent(c, []byte(tt.body))
			if got != tt.expected {
				t.Errorf("HasImageContent() = %v, want %v", got, tt.expected)
			}
		})
	}
}

func TestHasImageContent_Gemini(t *testing.T) {
	tests := []struct {
		name     string
		body     string
		expected bool
	}{
		{
			name:     "gemini inlineData",
			body:     `{"contents":[{"parts":[{"inlineData":{"mimeType":"image/png","data":"abc"}}]}]}`,
			expected: true,
		},
		{
			name:     "gemini fileData",
			body:     `{"contents":[{"parts":[{"fileData":{"mimeType":"image/jpeg","fileUri":"gs://bucket/img.jpg"}}]}]}`,
			expected: true,
		},
		{
			name:     "gemini text only",
			body:     `{"contents":[{"parts":[{"text":"hello"}]}]}`,
			expected: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := newTestContext()
			got := HasImageContent(c, []byte(tt.body))
			if got != tt.expected {
				t.Errorf("HasImageContent() = %v, want %v", got, tt.expected)
			}
		})
	}
}

func TestHasImageContent_EdgeCases(t *testing.T) {
	tests := []struct {
		name     string
		body     string
		expected bool
	}{
		{
			name:     "empty body",
			body:     "",
			expected: false,
		},
		{
			name:     "empty json",
			body:     "{}",
			expected: false,
		},
		{
			name:     "malformed json",
			body:     "{invalid",
			expected: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := newTestContext()
			got := HasImageContent(c, []byte(tt.body))
			if got != tt.expected {
				t.Errorf("HasImageContent() = %v, want %v", got, tt.expected)
			}
		})
	}
}

func TestHasImageContent_ContextCaching(t *testing.T) {
	c := newTestContext()
	body := []byte(`{"messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64","data":"abc"}}]}]}`)

	result1 := HasImageContent(c, body)
	if !result1 {
		t.Fatal("first call should detect image")
	}

	// 第二次调用即使传空 body 也应返回缓存结果
	result2 := HasImageContent(c, nil)
	if !result2 {
		t.Fatal("second call should return cached result")
	}
}
