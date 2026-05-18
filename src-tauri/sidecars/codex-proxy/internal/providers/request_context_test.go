package providers

import (
	"bytes"
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

type testContextKey string

func newGinContext(method, url string, body []byte, ctx context.Context) *gin.Context {
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)

	req := httptest.NewRequest(method, url, bytes.NewReader(body))
	if ctx != nil {
		req = req.WithContext(ctx)
	}
	c.Request = req
	return c
}

func TestConvertToProviderRequest_PropagatesContext(t *testing.T) {
	gin.SetMode(gin.TestMode)

	key := testContextKey("test-key")
	ctx := context.WithValue(context.Background(), key, "ok")

	t.Run("claude", func(t *testing.T) {
		c := newGinContext(http.MethodPost, "/v1/messages", []byte(`{"model":"claude-3","messages":[]}`), ctx)
		upstream := &config.UpstreamConfig{BaseURL: "https://api.example.com", ServiceType: "claude"}

		p := &ClaudeProvider{}
		req, _, err := p.ConvertToProviderRequest(c, upstream, "sk-ant-test")
		if err != nil {
			t.Fatalf("ConvertToProviderRequest() err = %v", err)
		}
		if got := req.Context().Value(key); got != "ok" {
			t.Fatalf("req.Context().Value(key) = %v, want %v", got, "ok")
		}
	})

	t.Run("openai", func(t *testing.T) {
		c := newGinContext(http.MethodPost, "/v1/messages", []byte(`{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}`), ctx)
		upstream := &config.UpstreamConfig{BaseURL: "https://api.example.com", ServiceType: "openai"}

		p := &OpenAIProvider{}
		req, _, err := p.ConvertToProviderRequest(c, upstream, "sk-test")
		if err != nil {
			t.Fatalf("ConvertToProviderRequest() err = %v", err)
		}
		if got := req.Context().Value(key); got != "ok" {
			t.Fatalf("req.Context().Value(key) = %v, want %v", got, "ok")
		}
	})

	t.Run("gemini", func(t *testing.T) {
		c := newGinContext(http.MethodPost, "/v1/messages", []byte(`{"model":"gemini-2.0-flash","messages":[{"role":"user","content":"hi"}]}`), ctx)
		upstream := &config.UpstreamConfig{BaseURL: "https://api.example.com", ServiceType: "gemini"}

		p := &GeminiProvider{}
		req, _, err := p.ConvertToProviderRequest(c, upstream, "AIza-test")
		if err != nil {
			t.Fatalf("ConvertToProviderRequest() err = %v", err)
		}
		if got := req.Context().Value(key); got != "ok" {
			t.Fatalf("req.Context().Value(key) = %v, want %v", got, "ok")
		}
	})

	t.Run("responses", func(t *testing.T) {
		c := newGinContext(http.MethodPost, "/v1/responses", []byte(`{"model":"gpt-4o","input":"hi"}`), ctx)
		upstream := &config.UpstreamConfig{BaseURL: "https://api.example.com", ServiceType: "responses"}

		p := &ResponsesProvider{}
		req, _, err := p.ConvertToProviderRequest(c, upstream, "sk-test")
		if err != nil {
			t.Fatalf("ConvertToProviderRequest() err = %v", err)
		}
		if got := req.Context().Value(key); got != "ok" {
			t.Fatalf("req.Context().Value(key) = %v, want %v", got, "ok")
		}
	})
}

func TestConvertToProviderRequest_UsesUpdatedRequestBodyBytesContext(t *testing.T) {
	gin.SetMode(gin.TestMode)
	body := []byte(`{"model":"claude-3","messages":[],"metadata":{"user_id":"{\"device_id\":\"abc\"}"}}`)
	normalized := []byte(`{"model":"claude-3","messages":[],"metadata":{"user_id":"user_abc"}}`)
	c := newGinContext(http.MethodPost, "/v1/messages", body, context.Background())
	c.Set("requestBodyBytes", normalized)
	upstream := &config.UpstreamConfig{BaseURL: "https://api.example.com", ServiceType: "claude"}

	p := &ClaudeProvider{}
	_, reqBody, err := p.ConvertToProviderRequest(c, upstream, "sk-ant-test")
	if err != nil {
		t.Fatalf("ConvertToProviderRequest() err = %v", err)
	}
	if string(reqBody) != string(normalized) {
		t.Fatalf("request body = %s, want %s", string(reqBody), string(normalized))
	}
}
