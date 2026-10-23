package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

func TestUsageTierKeepsCodexOAuthRequestAndResponseSeparate(t *testing.T) {
	for _, tc := range []struct {
		name, authType, request, response, want string
	}{
		{"OAuth Fast with default echo", "oauth", "priority", "default", "priority"},
		{"OAuth Fast with auto echo", "oauth", "priority", "auto", "priority"},
		{"OAuth Standard with priority echo", "oauth", "default", "priority", "standard"},
		{"API key downgrade", "api_key", "priority", "default", "standard"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			tracker := newRequestUsageTracker()
			plugin := &usagePlugin{tracker: tracker, defaultServiceTier: "priority"}
			ctx := internallogging.WithRequestID(context.Background(), "tier")
			plugin.HandleUsage(ctx, coreusage.Record{
				Provider: "codex", AuthType: tc.authType, ServiceTier: tc.request,
				UpstreamServiceTier: tc.request, UpstreamServiceTierObserved: true,
				ResponseServiceTier: tc.response,
			})
			got, ok := tracker.finalize("tier", usageFinalizeInput{status: http.StatusOK})
			if !ok || got.ServiceTier != tc.want || got.RequestedServiceTier != tc.request || got.ResponseServiceTier != tc.response {
				t.Fatalf("lost tier evidence: %+v", got)
			}
			encoded, err := json.Marshal(got)
			if err != nil || !strings.Contains(string(encoded), `"requestedServiceTier":"`+tc.request+`"`) ||
				!strings.Contains(string(encoded), `"responseServiceTier":"`+tc.response+`"`) {
				t.Fatalf("wire tier evidence = %s, err=%v", encoded, err)
			}
		})
	}
}

func TestObservedMissingUpstreamTierDoesNotInventFast(t *testing.T) {
	record := coreusage.Record{Provider: "codex", AuthType: "oauth", ServiceTier: "priority", UpstreamServiceTierObserved: true}
	if got := requestedUsageServiceTier(record, "priority"); got != "auto" {
		t.Fatalf("missing serialized field inferred as %q", got)
	}
	if got := usageServiceTier(record, "priority"); got != "" {
		t.Fatalf("unconfirmed processing tier = %q", got)
	}
}

func TestTierAuthAttributionFallsBackToMatchedAccount(t *testing.T) {
	tracker := newRequestUsageTracker()
	plugin := &usagePlugin{tracker: tracker, manifest: &manifest{accountByAuthID: map[string]*accountSpec{
		"oauth": {ID: "a", AuthKind: "oauth", Provider: "codex"},
	}}}
	ctx := internallogging.WithRequestID(context.Background(), "tier")
	plugin.HandleUsage(ctx, coreusage.Record{AuthID: "oauth", ServiceTier: "priority", ResponseServiceTier: "default"})
	got, _ := tracker.finalize("tier", usageFinalizeInput{status: http.StatusOK})
	if got.ServiceTier != "priority" || got.ResponseServiceTier != "default" || got.AccountID != "a" {
		t.Fatalf("account route evidence lost: %+v", got)
	}
	if got.RequestedServiceTier != "" {
		t.Fatalf("uncaptured upstream payload invented evidence: %+v", got)
	}
}

func TestDisabledResponsesWebsocketIsDiagnosticOnly(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, tc := range []struct {
		name                       string
		upgrade, validKey, enabled bool
		tokenLimit                 uint64
		status, usageCount         int
	}{
		{"disabled protocol fallback", true, true, false, 0, http.StatusUpgradeRequired, 0},
		{"plain GET still fails", false, true, false, 0, http.StatusUpgradeRequired, 1},
		{"invalid authentication still fails", true, false, false, 0, http.StatusUnauthorized, 1},
		{"enabled unavailable handler still fails", true, true, true, 0, http.StatusServiceUnavailable, 1},
		{"token limit still fails", true, true, false, 1, http.StatusTooManyRequests, 1},
	} {
		t.Run(tc.name, func(t *testing.T) {
			m := &manifest{APIKeys: []apiKeySpec{{ID: "key", Key: "client-key", Enabled: true,
				ResponsesWebsockets: tc.enabled, TokenLimit: tc.tokenLimit, TokenUsed: tc.tokenLimit}}}
			m.apiKeyByValue = map[string]*apiKeySpec{"client-key": &m.APIKeys[0]}
			tracker := newRequestUsageTracker()
			policy := &requestPolicy{manifest: m, tracker: tracker, emitter: &eventEmitter{}, tokenLimiter: newAPIKeyTokenLimiter(m)}
			router := (&relayServer{manifest: m, policy: policy, runtime: &fakeRuntime{}}).router()
			req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
			if tc.validKey {
				req.Header.Set("Authorization", "Bearer client-key")
			}
			if tc.upgrade {
				req.Header.Set("Upgrade", "websocket")
				req.Header.Set("Connection", "Upgrade")
			}
			response := httptest.NewRecorder()
			out := captureStdout(t, func() { router.ServeHTTP(response, req) })
			if response.Code != tc.status {
				t.Fatalf("status = %d body=%s", response.Code, response.Body.String())
			}
			usageCount, completedCount := 0, 0
			for _, line := range strings.Split(out, "\n") {
				var event struct {
					Type         string `json:"type"`
					Status       int    `json:"status"`
					ErrorMessage string `json:"errorMessage"`
				}
				if json.Unmarshal([]byte(line), &event) != nil {
					continue
				}
				if event.Type == "usage" {
					usageCount++
				}
				if event.Type == "request_completed" {
					completedCount++
					if event.Status != tc.status {
						t.Fatalf("diagnostic status lost: %+v", event)
					}
					if tc.usageCount == 0 && !strings.Contains(event.ErrorMessage, "responses websocket is disabled") {
						t.Fatalf("disabled reason missing from diagnostics: %+v", event)
					}
				}
			}
			if usageCount != tc.usageCount || completedCount != 1 {
				t.Fatalf("usage=%d completed=%d output=%s", usageCount, completedCount, out)
			}
			if len(tracker.records) != 0 || len(tracker.selectedAccounts) != 0 {
				t.Fatal("handshake left request state behind")
			}
		})
	}
}
