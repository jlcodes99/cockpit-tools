package main

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestResponsesProviderPerModelVisionPreservesExplicitChoice(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, supports := range []bool{true, false} {
		t.Run(map[bool]string{true: "enabled", false: "disabled"}[supports], func(t *testing.T) {
			var forwarded string
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/v1/responses" {
					t.Errorf("unexpected protocol path: %s", r.URL.Path)
				}
				body, _ := io.ReadAll(r.Body)
				forwarded = string(body)
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write([]byte(`{"id":"resp-test","object":"response","status":"completed","output":[]}`))
			}))
			defer upstream.Close()
			gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "test-upstream-key",
				UpstreamModel: "vision-model", UpstreamModels: []string{"vision-model"}, WireAPI: "responses",
				SupportsVision:    !supports,
				ModelCapabilities: map[string]providerGatewayModelCapability{"vision-model": {SupportsVision: supports}},
			}
			spec := apiKeySpec{ID: "test-key", Key: "test-client", Enabled: true, ProviderGateway: gateway}
			m := &manifest{APIKeys: []apiKeySpec{spec}, ModelIDs: []string{"vision-model"}, apiKeyByValue: map[string]*apiKeySpec{"test-client": &spec}}
			router := (&relayServer{runtime: &fakeRuntime{}, cfg: &config.Config{}, manifest: m, policy: &requestPolicy{manifest: m}}).router()
			req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"vision-model","input":[{"role":"user","content":[{"type":"input_text","text":"describe"},{"type":"input_image","image_url":"data:image/png;base64,abc"}]}],"stream":false}`))
			req.Header.Set("Authorization", "Bearer test-client")
			req.Header.Set("Content-Type", "application/json")
			response := httptest.NewRecorder()
			router.ServeHTTP(response, req)
			if response.Code != http.StatusOK {
				t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
			}
			if strings.Contains(forwarded, "data:image/png;base64,abc") != supports {
				t.Fatalf("unexpected image handling: %s", forwarded)
			}
			if strings.Contains(forwarded, providerGatewayOmittedImageText) == supports {
				t.Fatalf("unexpected omission notice: %s", forwarded)
			}
		})
	}
}
