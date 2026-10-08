package main

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestCodexBackendVoiceAliases(t *testing.T) {
	spec := &apiKeySpec{ID: "voice", Key: "voice-key", Enabled: true, AllowedModels: []string{"gpt-5.5"}}
	m := &manifest{apiKeyByValue: map[string]*apiKeySpec{spec.Key: spec}}
	router := (&relayServer{manifest: m, policy: &requestPolicy{manifest: m, tokenLimiter: newAPIKeyTokenLimiter(m)}}).router()
	for _, endpoint := range []struct{ method, path string }{
		{http.MethodPost, "/backend-api/codex/realtime/calls?intent=quicksilver&architecture=avas"},
		{http.MethodGet, "/backend-api/codex/rtc_test"},
		{http.MethodGet, "/backend-api/codex/realtime/calls/rtc_test"},
	} {
		for _, authorized := range []bool{false, true} {
			req := httptest.NewRequest(endpoint.method, endpoint.path, strings.NewReader(`{"model":"gpt-live-1-codex","session":{"model":"gpt-live-1-codex"},"sdp":"v=0"}`))
			req.Header.Set("Content-Type", "application/json")
			want := http.StatusUnauthorized
			if authorized {
				req.Header.Set("Authorization", "Bearer "+spec.Key)
				want = http.StatusServiceUnavailable
			}
			w := httptest.NewRecorder()
			router.ServeHTTP(w, req)
			if w.Code != want {
				t.Fatalf("%s %s authorized=%v: status=%d want=%d body=%s", endpoint.method, endpoint.path, authorized, w.Code, want, w.Body.String())
			}
		}
	}
}
