package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestAuthDiagnosticsCarryOriginalRequestStart(t *testing.T) {
	s, auths := rotationSelectorForTest()
	s.emitter = &eventEmitter{}
	ctx := context.WithValue(context.Background(), requestStartedAtContextKey, int64(123456))
	output := captureStdout(t, func() {
		s.emitAuthSelected(ctx, auths[0], "codex", "gpt-5.4", 3, 3)
		s.emitAuthPoolUnavailable(ctx, "codex", "gpt-5.4", authPoolSelectionStats{candidateAuths: 3}, &coreauth.Error{Code: "auth_pool_unavailable"})
		(&authHook{manifest: s.manifest, emitter: s.emitter}).OnResult(ctx, coreauth.Result{AuthID: auths[0].ID, Provider: "codex", Success: true})
	})
	lines := strings.Split(strings.TrimSpace(output), "\n")
	if len(lines) != 3 {
		t.Fatalf("expected three diagnostics, got %q", output)
	}
	for _, line := range lines {
		var payload requestDiagnosticPayload
		if err := json.Unmarshal([]byte(line), &payload); err != nil || payload.StartedAtMS != 123456 {
			t.Fatalf("diagnostic lost original start: %#v err=%v", payload, err)
		}
	}
	if requestStartedAtMS(nil) != 0 || requestStartedAtMS(context.Background()) != 0 {
		t.Fatal("unknown request starts must remain unknown")
	}
}

func TestRequestPolicyCarriesRequestStartThroughHTTPAndWebsocket(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, transport := range []string{"http", "websocket"} {
		t.Run(transport, func(t *testing.T) {
			policy := &requestPolicy{manifest: &manifest{}, tokenLimiter: newAPIKeyTokenLimiter(nil)}
			router := gin.New()
			router.Use(policy.middleware())
			var start int64
			router.GET("/v1/responses", func(c *gin.Context) {
				start = requestStartedAtMS(c.Request.Context())
				c.Status(http.StatusOK)
			})
			req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
			if transport == "websocket" {
				req.Header.Set("Upgrade", "websocket")
				req.Header.Set("Connection", "Upgrade")
			}
			before := time.Now().UnixMilli()
			response := httptest.NewRecorder()
			router.ServeHTTP(response, req)
			if response.Code != http.StatusOK || start < before || start > time.Now().UnixMilli() {
				t.Fatalf("request start missing: status=%d start=%d before=%d", response.Code, start, before)
			}
		})
	}
}
