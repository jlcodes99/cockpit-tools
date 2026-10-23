package main

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

type priorityScopeRuntime struct {
	fakeRuntime
	scopes [][]string
}

func (r *priorityScopeRuntime) Execute(ctx context.Context, providers []string, req coreexecutor.Request, opts coreexecutor.Options) (coreexecutor.Response, error) {
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	if spec != nil {
		r.scopes = append(r.scopes, append([]string(nil), spec.AccountIDs...))
	}
	return r.fakeRuntime.Execute(ctx, providers, req, opts)
}

func TestCustomPriorityCrossesNativeAndProviderRoutesForEveryKey(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, keyID := range []string{"default", "team-a", "team-b"} {
		for _, providerFails := range []bool{false, true} {
			t.Run(keyID+map[bool]string{false: "/preferred", true: "/fallback"}[providerFails], func(t *testing.T) {
				providerCalls := 0
				upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
					providerCalls++
					w.Header().Set("Content-Type", "application/json")
					if providerFails {
						w.WriteHeader(http.StatusServiceUnavailable)
						_, _ = io.WriteString(w, `{"error":{"message":"synthetic unavailable"}}`)
						return
					}
					_, _ = io.WriteString(w, `{"id":"provider-priority"}`)
				}))
				defer upstream.Close()
				m := loadAutomaticRoutingManifest(t, strings.Replace(automaticRoutingManifestPayload, "http://127.0.0.1:1/v1", upstream.URL, 1))
				m.RoutingStrategy = "custom"
				m.CustomRoutingRules = []customRoutingRule{{AccountID: "chat-account", Priority: 10, IsPreferred: true}, {AccountID: "native-account", Priority: 1}}
				spec := m.apiKeyByValue["client-key"]
				spec.ID = keyID
				runtime := &priorityScopeRuntime{fakeRuntime: fakeRuntime{response: coreexecutor.Response{Payload: []byte(`{"id":"native-fallback"}`)}}}
				server := &relayServer{manifest: m, runtime: runtime, cfg: &config.Config{}, policy: &requestPolicy{manifest: m, tracker: newRequestUsageTracker()}, automaticSelector: &firstAuthSelector{}}
				response := postAutomaticRoutingRequest(t, server.router(), "shared")
				if response.Code != http.StatusOK || providerCalls != 1 {
					t.Fatalf("preferred provider was skipped: status=%d calls=%d body=%s", response.Code, providerCalls, response.Body.String())
				}
				if !providerFails && (runtime.executeCalls != 0 || !strings.Contains(response.Body.String(), "provider-priority")) {
					t.Fatalf("native path overrode the preferred provider: calls=%d body=%s", runtime.executeCalls, response.Body.String())
				}
				if providerFails && (runtime.executeCalls != 1 || !strings.Contains(response.Body.String(), "native-fallback") || len(runtime.scopes) != 1 || strings.Join(runtime.scopes[0], ",") != "native-account") {
					t.Fatalf("fallback did not retain the scoped lower tier: calls=%d scopes=%v body=%s", runtime.executeCalls, runtime.scopes, response.Body.String())
				}
				if len(spec.AccountIDs) != 2 || len(spec.ModelRouting.Routes) != 1 {
					t.Fatal("per-request priority mutated the shared key configuration")
				}
			})
		}
	}
}

func TestCustomAutomaticTiersKeepKeyPriorityAndScope(t *testing.T) {
	m := loadAutomaticRoutingManifest(t, automaticRoutingManifestPayload)
	m.RoutingStrategy = "custom"
	m.CustomRoutingRules = []customRoutingRule{{AccountID: "native-account", Priority: 10}, {AccountID: "chat-account", Priority: 1}}
	path := filepath.Join(t.TempDir(), "api-key-priorities.json")
	if err := os.WriteFile(path, []byte(`{"priorityAccountIds":{"client":["outside-scope","chat-account"]}}`), 0600); err != nil {
		t.Fatal(err)
	}
	s := &relayServer{manifest: m, priorities: newAPIKeyPriorityStateStore(filepath.Join(filepath.Dir(path), "manifest.json"))}
	spec := m.apiKeyByValue["client-key"]
	tiers := s.customAutomaticTiers(spec, "shared")
	if len(tiers) != 2 || strings.Join(tiers[0].AccountIDs, ",") != "chat-account" || strings.Join(tiers[1].AccountIDs, ",") != "native-account" {
		t.Fatalf("key override or authorization scope lost: %+v", tiers)
	}
}

func TestCustomPrioritySkipsPausedPreferredProvider(t *testing.T) {
	gin.SetMode(gin.TestMode)
	providerCalls := 0
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		providerCalls++
		_, _ = io.WriteString(w, `{"id":"paused-provider"}`)
	}))
	defer upstream.Close()
	m := loadAutomaticRoutingManifest(t, strings.Replace(automaticRoutingManifestPayload, "http://127.0.0.1:1/v1", upstream.URL, 1))
	m.RoutingStrategy = "custom"
	m.PausedAccountIDs = []string{"chat-account"}
	m.CustomRoutingRules = []customRoutingRule{{AccountID: "chat-account", IsPreferred: true}}
	runtime := &priorityScopeRuntime{fakeRuntime: fakeRuntime{response: coreexecutor.Response{Payload: []byte(`{"id":"active-native"}`)}}}
	server := &relayServer{manifest: m, runtime: runtime, cfg: &config.Config{}, policy: &requestPolicy{manifest: m, tracker: newRequestUsageTracker()}}
	response := postAutomaticRoutingRequest(t, server.router(), "shared")
	if response.Code != 200 || providerCalls != 0 || runtime.executeCalls != 1 || len(runtime.scopes) != 1 || strings.Join(runtime.scopes[0], ",") != "native-account" {
		t.Fatalf("priority bypassed service pause or scope: status=%d provider=%d scopes=%v body=%s", response.Code, providerCalls, runtime.scopes, response.Body.String())
	}
}

func TestCustomPriorityNeverReplaysCommittedStream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"}}]}\n\n")
		w.(http.Flusher).Flush()
		// EOF before a terminal frame must not generate a second answer.
	}))
	defer upstream.Close()
	m := loadAutomaticRoutingManifest(t, strings.Replace(automaticRoutingManifestPayload, "http://127.0.0.1:1/v1", upstream.URL, 1))
	m.RoutingStrategy = "custom"
	m.CustomRoutingRules = []customRoutingRule{{AccountID: "chat-account", IsPreferred: true}}
	runtime := &priorityScopeRuntime{fakeRuntime: fakeRuntime{response: coreexecutor.Response{Payload: []byte(`{"id":"unexpected-replay"}`)}}}
	server := &relayServer{manifest: m, runtime: runtime, cfg: &config.Config{}, policy: &requestPolicy{manifest: m, tracker: newRequestUsageTracker()}}
	req := httptest.NewRequest("POST", "/v1/responses", strings.NewReader(`{"model":"shared","input":"fixture","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	server.router().ServeHTTP(response, req)
	if response.Code != 200 || !strings.Contains(response.Body.String(), "partial") || runtime.executeCalls != 0 {
		t.Fatalf("committed stream replayed: status=%d native=%d body=%s", response.Code, runtime.executeCalls, response.Body.String())
	}
}
