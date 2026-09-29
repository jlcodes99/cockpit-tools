package main

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestProviderGatewayCompactIsPerKey(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, test := range []struct {
		name    string
		remote  bool
		wireAPI string
		status  int
	}{
		{"remote responses key", true, "responses", http.StatusOK},
		{"local responses key", false, "responses", http.StatusNotFound},
		{"chat key", true, "chat_completions", http.StatusNotFound},
	} {
		t.Run(test.name, func(t *testing.T) {
			forwarded := 0
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				forwarded++
				if r.URL.Path != "/v1/responses/compact" || r.Header.Get("Authorization") != "Bearer upstream-key" {
					t.Errorf("unexpected upstream request: path=%s auth=%s", r.URL.Path, r.Header.Get("Authorization"))
				}
				body, _ := io.ReadAll(r.Body)
				var payload map[string]any
				if err := json.Unmarshal(body, &payload); err != nil || payload["model"] != "real-model" || payload["input"] != "history" {
					t.Errorf("unexpected compact body: %s (%v)", body, err)
				}
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write([]byte(`{"output":[{"type":"compaction","encrypted_content":"test"}]}`))
			}))
			defer upstream.Close()
			gateway := &providerGatewaySpec{
				BaseURL: upstream.URL + "/v1", APIKey: "upstream-key",
				UpstreamModel: "real-model", UpstreamModels: []string{"real-model"},
				WireAPI: test.wireAPI, SupportsRemoteCompaction: test.remote,
			}
			spec := apiKeySpec{ID: "client", Key: "client-key", Enabled: true, ProviderGateway: gateway}
			m := &manifest{APIKeys: []apiKeySpec{spec}, ModelIDs: []string{"alias"}, apiKeyByValue: map[string]*apiKeySpec{"client-key": &spec}}
			router := (&relayServer{runtime: &fakeRuntime{}, cfg: &config.Config{}, manifest: m, policy: &requestPolicy{manifest: m}}).router()
			req := httptest.NewRequest(http.MethodPost, "/v1/responses/compact", strings.NewReader(`{"model":"alias","input":"history"}`))
			req.Header.Set("Authorization", "Bearer client-key")
			req.Header.Set("Content-Type", "application/json")
			response := httptest.NewRecorder()
			router.ServeHTTP(response, req)
			if response.Code != test.status {
				t.Fatalf("status=%d, want=%d, body=%s", response.Code, test.status, response.Body.String())
			}
			wantForwarded := 0
			if test.status == http.StatusOK {
				wantForwarded = 1
			}
			if forwarded != wantForwarded {
				t.Fatalf("forwarded=%d, status=%d", forwarded, response.Code)
			}
		})
	}
}
