package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestPausedDirectProviderRoutesDoNotReachUpstream(t *testing.T) {
	for _, named := range []bool{false, true} {
		for _, stream := range []bool{false, true} {
			t.Run(fmt.Sprintf("named=%t/stream=%t", named, stream), func(t *testing.T) {
				var mu sync.Mutex
				calls := 0
				upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					mu.Lock()
					calls++
					mu.Unlock()
					_, _ = io.WriteString(w, `{"id":"must-not-call"}`)
				}))
				defer upstream.Close()
				gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "fixture-a", UpstreamModel: "provider-model", UpstreamModels: []string{"provider-model"}, WireAPI: "chat_completions"}
				spec := apiKeySpec{ID: "client", Key: "client-key", Enabled: true, AccountIDs: []string{"a"}, ProviderGateway: gateway}
				model := "provider-model"
				if named {
					// A named route has its own authorized member, independent of the default pool.
					spec.AccountIDs = []string{"b"}
					spec.ProviderGateway = nil
					spec.ModelRouting = &modelRoutingSpec{DefaultRoute: "oauth", FailurePolicy: "strict", Routes: []modelRouteSpec{{ID: "route-a", Namespace: "named", ProviderAccountID: "a", ProviderGateway: gateway}}}
					model = "named/provider-model"
				}
				payload, err := json.Marshal(manifest{PausedAccountIDs: []string{"a"}, Accounts: []accountSpec{{ID: "a"}, {ID: "b"}}, ModelIDs: []string{"provider-model"}, APIKeys: []apiKeySpec{spec}})
				if err != nil {
					t.Fatal(err)
				}
				m := loadAutomaticRoutingManifest(t, string(payload))
				router := (&relayServer{manifest: m, cfg: &config.Config{}, policy: &requestPolicy{manifest: m}}).router()
				r := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(fmt.Sprintf(`{"model":%q,"messages":[{"role":"user","content":"fixture"}],"stream":%t}`, model, stream)))
				r.Header.Set("Authorization", "Bearer client-key")
				r.Header.Set("Content-Type", "application/json")
				w := httptest.NewRecorder()
				router.ServeHTTP(w, r)
				mu.Lock()
				gotCalls := calls
				mu.Unlock()
				if w.Code != 503 || gotCalls != 0 || !strings.Contains(w.Body.String(), "account_paused") {
					t.Fatalf("paused direct route escaped: status=%d calls=%d body=%s", w.Code, gotCalls, w.Body.String())
				}
				if strings.Join(m.APIKeys[0].AccountIDs, ",") != strings.Join(spec.AccountIDs, ",") || len(m.Accounts) != 2 {
					t.Fatal("scope or members changed")
				}
			})
		}
	}
}

func TestPausedAutomaticProviderFallbackStaysInsideActiveScope(t *testing.T) {
	for _, allPaused := range []bool{false, true} {
		t.Run(fmt.Sprint(allPaused), func(t *testing.T) {
			gin.SetMode(gin.TestMode)
			var mu sync.Mutex
			var called []string
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				mu.Lock()
				called = append(called, strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer fixture-"))
				mu.Unlock()
				w.Header().Set("Content-Type", "application/json")
				_, _ = io.WriteString(w, `{"id":"provider-active"}`)
			}))
			defer upstream.Close()
			spec := apiKeySpec{ID: "client", Key: "client-key", Enabled: true, AccountIDs: []string{"a", "b"}, ModelRouting: &modelRoutingSpec{Automatic: true, DefaultRoute: "oauth", FailurePolicy: "strict", NativeModels: []string{"shared"}}}
			for _, id := range []string{"a", "b", "outside"} {
				spec.ModelRouting.Routes = append(spec.ModelRouting.Routes, modelRouteSpec{ID: "route-" + id, Namespace: id, ProviderAccountID: id, ProviderGateway: &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "fixture-" + id, UpstreamModel: "provider-model", UpstreamModels: []string{"provider-model"}, WireAPI: "chat_completions"}, Models: []modelRouteModelSpec{{ClientModel: "shared", UpstreamModel: "provider-model"}}})
			}
			paused := []string{"a"}
			if allPaused {
				paused = append(paused, "b")
			}
			payload, err := json.Marshal(manifest{PausedAccountIDs: paused, Accounts: []accountSpec{{ID: "a"}, {ID: "b"}, {ID: "outside"}}, ModelIDs: []string{"shared"}, APIKeys: []apiKeySpec{spec}})
			if err != nil {
				t.Fatal(err)
			}
			m := loadAutomaticRoutingManifest(t, string(payload))
			runtime := &fakeRuntime{err: errors.New("fixture native unavailable")}
			router := (&relayServer{manifest: m, cfg: &config.Config{}, runtime: runtime, policy: &requestPolicy{manifest: m}}).router()
			w := postAutomaticRoutingRequest(t, router, "shared")
			mu.Lock()
			got := strings.Join(called, ",")
			mu.Unlock()
			if runtime.executeCalls != 1 {
				t.Fatalf("native attempt missing: %d", runtime.executeCalls)
			}
			if allPaused {
				if w.Code < 400 || got != "" {
					t.Fatalf("paused automatic scope leaked: status=%d called=%s body=%s", w.Code, got, w.Body.String())
				}
			} else if w.Code != 200 || got != "b" || !strings.Contains(w.Body.String(), "provider-active") {
				t.Fatalf("active provider fallback failed: status=%d called=%s body=%s", w.Code, got, w.Body.String())
			}
		})
	}
}

func TestPausedNativeNamedRouteDoesNotFallBackToDefaultPool(t *testing.T) {
	router, runtime, spec := newNativeRouteTestRouter()
	runtime.selector.manifest.PausedAccountIDs = []string{"grok-target"}
	r := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(`{"model":"grok/grok-4.6","messages":[{"role":"user","content":"fixture"}]}`))
	r.Header.Set("Authorization", "Bearer "+spec.Key)
	r.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, r)
	if w.Code != 503 || runtime.selected != nil || runtime.executeCalls != 0 || strings.Join(spec.AccountIDs, ",") != "oauth-default" {
		t.Fatalf("paused native route escaped or changed scope: status=%d selected=%#v calls=%d scope=%v body=%s", w.Code, runtime.selected, runtime.executeCalls, spec.AccountIDs, w.Body.String())
	}
}
