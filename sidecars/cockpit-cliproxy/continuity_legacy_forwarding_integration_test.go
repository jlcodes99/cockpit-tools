//go:build continuity_integration

package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	runtimeexecutor "github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/tidwall/gjson"
)

func legacyForwardingContinuityRouter(t *testing.T, upstream string) http.Handler {
	t.Helper()
	executable := os.Getenv("COCKPIT_CONTINUITY_TEST_EXE")
	if executable == "" {
		t.Fatal("build the Rust host and set COCKPIT_CONTINUITY_TEST_EXE")
	}
	gin.SetMode(gin.TestMode)
	m := continuityManifestFixture(t)
	key := &apiKeySpec{ID: "legacy", Key: "legacy-forwarding-client", Enabled: true, AccountIDs: []string{"local-a", "local-b"}}
	m.apiKeyByValue = map[string]*apiKeySpec{key.Key: key}
	m.ModelIDs = []string{"gpt-5.4"}
	cfg := &config.Config{}
	cfg.Codex.StreamBootstrapBuffering = true
	tracker := newRequestUsageTracker()
	manager := coreauth.NewManager(nil, &accountSlotSelector{manifest: m, tracker: tracker, fallback: &orderedAuthSelector{order: []string{"sdk-a", "sdk-b"}}}, &authHook{manifest: m})
	manager.SetConfig(cfg)
	manager.SetRetryConfig(0, 0, 2)
	manager.RegisterExecutor(runtimeexecutor.NewCodexExecutor(cfg))
	for _, id := range []string{"a", "b"} {
		authID := "sdk-" + id
		registry.GetGlobalRegistry().RegisterClient(authID, "codex", []*registry.ModelInfo{{ID: "gpt-5.4"}})
		t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(authID) })
		if _, err := manager.Register(context.Background(), &coreauth.Auth{
			ID: authID, Provider: "codex", Status: coreauth.StatusActive,
			Attributes: map[string]string{"account_id": "local-" + id, "api_key": "upstream-" + id, "base_url": upstream},
		}); err != nil {
			t.Fatal(err)
		}
	}
	m.authManager = manager
	directory := t.TempDir()
	token := strings.Repeat("e", 64)
	configPath := filepath.Join(directory, "continuity.json")
	rawConfig, err := json.Marshal(map[string]any{
		"identity_path": filepath.Join(directory, "identity.sqlite"), "token": token,
		"owners":          map[string]string{"local-a": "owner-a", "local-b": "owner-b"},
		"caller_accounts": map[string][]string{"native:primary": {"local-a", "local-b"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(configPath, rawConfig, 0600); err != nil {
		t.Fatal(err)
	}
	m.continuityClient, err = startContinuityWorker(context.Background(), &continuityServiceSpec{
		Version: 1, Executable: executable, ConfigPath: configPath, Token: token,
	}, m)
	if err != nil {
		t.Fatal("headless worker startup", err)
	}
	t.Cleanup(func() { m.continuityClient.Close() })
	return (&relayServer{
		manifest: m, policy: &requestPolicy{manifest: m, tracker: tracker},
		runtime: manager, authManager: manager, cfg: cfg,
	}).router()
}

func TestRealContinuityWorkerLegacyForwardingPreservesHistoryAndUpstreamErrors(t *testing.T) {
	for _, status := range []int{http.StatusOK, http.StatusTooManyRequests, http.StatusServiceUnavailable} {
		t.Run(fmt.Sprintf("upstream_%d", status), func(t *testing.T) {
			var mu sync.Mutex
			var bodies []map[string]any
			var accounts []string
			errorCode := "usage_limit_reached"
			if status == http.StatusServiceUnavailable {
				errorCode = "server_is_overloaded"
			}
			upstreamError := map[string]any{
				"type": "server_error", "code": errorCode,
				"message": fmt.Sprintf("Original loopback upstream failure %d", status),
				"param":   "input", "upstream_marker": "preserve-this-upstream-diagnostic",
			}
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				var body map[string]any
				if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
					t.Error(err)
					w.WriteHeader(http.StatusBadRequest)
					return
				}
				account := r.Header.Get("Authorization")
				mu.Lock()
				bodies = append(bodies, body)
				accounts = append(accounts, account)
				mu.Unlock()
				if status != http.StatusOK && account == "Bearer upstream-a" {
					w.Header().Set("Content-Type", "application/json")
					w.WriteHeader(status)
					_ = json.NewEncoder(w).Encode(map[string]any{"error": upstreamError})
					return
				}
				// B would succeed if the gateway incorrectly replayed this history.
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = fmt.Fprint(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"legacy-upstream-success\",\"status\":\"completed\",\"output\":[]}}\n\n")
			}))
			defer upstream.Close()
			router := legacyForwardingContinuityRouter(t, upstream.URL)
			input := []any{
				map[string]any{"role": "user", "content": "continue the original task"},
				map[string]any{"type": "reasoning", "summary": []any{}, "encrypted_content": "unregistered-legacy-cipher-preserve-exactly"},
				map[string]any{"type": "function_call_output", "call_id": "orphan-result", "output": "Original result\nwith whitespace.  "},
				map[string]any{"type": "future_history_item", "payload": map[string]any{"keep": "exact extension"}},
			}
			body, err := json.Marshal(map[string]any{
				"model": "gpt-5.4", "stream": true, "input": input,
				"client_metadata": map[string]any{"session_id": "legacy-root", "thread_id": "legacy-root"},
			})
			if err != nil {
				t.Fatal(err)
			}
			request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(string(body)))
			request.Header.Set("Authorization", "Bearer legacy-forwarding-client")
			request.Header.Set("Content-Type", "application/json")
			request.Header.Set("User-Agent", "codex-tui/0.145.0")
			response := httptest.NewRecorder()
			router.ServeHTTP(response, request)
			if response.Code != status {
				t.Fatalf("status=%d want=%d body=%s", response.Code, status, response.Body.String())
			}
			if status == http.StatusOK {
				if !strings.Contains(response.Body.String(), "legacy-upstream-success") {
					t.Fatalf("upstream success did not reach client: %s", response.Body.String())
				}
			} else {
				actual := gjson.Get(response.Body.String(), "error")
				for _, field := range []string{"code", "message", "param", "upstream_marker"} {
					if actual.Get(field).String() != upstreamError[field] {
						t.Errorf("upstream %s was replaced: %s", field, response.Body.String())
					}
				}
			}
			mu.Lock()
			defer mu.Unlock()
			if !reflect.DeepEqual(accounts, []string{"Bearer upstream-a"}) {
				t.Fatalf("unproven history was replayed across accounts: %#v", accounts)
			}
			if len(bodies) != 1 || !reflect.DeepEqual(bodies[0]["input"], input) {
				t.Fatalf("original history was blocked, repaired or retried: %#v", bodies)
			}
		})
	}
}
