package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func modelErrorTranslations() map[string]map[string]string {
	return map[string]map[string]string{"model_not_available": {
		"zh-CN": "模型 {{model}} 不可用。", "en": "Model {{model}} unavailable.", "pt": "Modelo {{model}} indisponível.",
	}}
}

func TestModelUnavailableMessageLocaleFallback(t *testing.T) {
	for _, tc := range []struct{ locale, want string }{
		{"zh-CN", "模型 gpt-test 不可用。"}, {" ZH-cn ", "模型 gpt-test 不可用。"},
		{"en-US", "Model gpt-test unavailable."}, {"en", "Model gpt-test unavailable."},
		{"pt-BR", "Modelo gpt-test indisponível."}, {"unknown", "Model gpt-test unavailable."},
	} {
		t.Run(tc.locale, func(t *testing.T) {
			m := &manifest{Locale: tc.locale, GatewayErrorMessages: modelErrorTranslations()}
			if got := m.modelUnavailableMessage("gpt-test"); got != tc.want {
				t.Fatalf("message = %q, want %q", got, tc.want)
			}
		})
	}
	for _, m := range []*manifest{nil, {}, {Locale: "zh-CN", GatewayErrorMessages: map[string]map[string]string{"model_not_available": {"zh-CN": "  "}}}} {
		if got := m.modelUnavailableMessage("gpt-test"); got != "Model gpt-test is not available for this API key." {
			t.Fatalf("missing/blank translations must use English fallback, got %q", got)
		}
	}
}

func TestModelUnavailableMessageAcrossRequestEntrypoints(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, locale := range []string{"zh-CN", "en-US", "pt-BR"} {
		for _, entry := range []string{"responses-policy", "alpha-search", "image", "automatic", "provider", "ollama"} {
			t.Run(locale+"/"+entry, func(t *testing.T) {
				key := &apiKeySpec{ID: "key", Key: "qa-key", Enabled: true, AllowedModels: []string{"allowed"}}
				m := &manifest{Locale: locale, GatewayErrorMessages: modelErrorTranslations(), ModelIDs: []string{"allowed"},
					apiKeyByValue: map[string]*apiKeySpec{key.Key: key}}
				server := &relayServer{manifest: m, policy: &requestPolicy{manifest: m}, runtime: &fakeRuntime{}}
				request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-test","input":"x","stream":false}`))
				request.Header.Set("Content-Type", "application/json")
				request.Header.Set("Authorization", "Bearer "+key.Key)
				// Request Accept-Language must never override the host locale.
				request.Header.Set("Accept-Language", "ja-JP")
				request = request.WithContext(context.WithValue(request.Context(), clientAPIKeyContextKey, key))
				w := httptest.NewRecorder()
				c, _ := gin.CreateTestContext(w)
				c.Request = request
				switch entry {
				case "responses-policy":
					server.router().ServeHTTP(w, request)
				case "alpha-search":
					server.handleCodexAlphaSearch(c)
				case "image":
					server.handleImagesRelayRequest(c, imageRelayRequest{requestedModel: "gpt-test"})
				case "automatic":
					key.ModelRouting = &modelRoutingSpec{Automatic: true, NativeModels: []string{"allowed"}}
					server.handleAutomaticModelRequest(c, key, nil, "gpt-test", sdktranslator.FormatOpenAIResponse, "")
				case "provider":
					server.handleProviderGatewayRequest(c, &providerGatewaySpec{UpstreamModels: []string{"allowed"}}, nil, "gpt-test", sdktranslator.FormatOpenAIResponse, "")
				case "ollama":
					server.handleOllamaProviderGatewayChat(c, &providerGatewaySpec{WireAPI: "chat_completions", UpstreamModels: []string{"allowed"}}, nil, "gpt-test", false)
				}
				var body struct {
					Error struct{ Code, Message string }
				}
				if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
					t.Fatal(err)
				}
				if w.Code != http.StatusNotFound || body.Error.Code != "model_not_available" || body.Error.Message != m.modelUnavailableMessage("gpt-test") {
					t.Fatalf("status=%d error=%+v", w.Code, body.Error)
				}
			})
		}
	}
}
