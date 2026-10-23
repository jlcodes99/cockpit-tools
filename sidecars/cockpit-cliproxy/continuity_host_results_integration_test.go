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
)

func hostResultHistory() []any {
	return []any{
		map[string]any{"role": "user", "content": "original task"},
		map[string]any{"type": "function_call", "name": "read_file", "call_id": "real-call", "arguments": "{}"},
		map[string]any{"type": "function_call_output", "namespace": "codex_app", "name": "create_thread", "output": "Original host result\n  exact text  "},
		map[string]any{"type": "function_call_output", "call_id": "real-call", "output": "original file contents"},
		map[string]any{"type": "function_call_output", "namespace": "codex_app", "name": "send_message_to_thread", "call_id": "", "output": []any{map[string]any{"type": "input_text", "text": "Host payload:"}, map[string]any{"type": "encrypted_content", "encrypted_content": "host-cipher"}}},
	}
}

func TestRealContinuityWorkerHostResultsChildAndRestart(t *testing.T) {
	runRealContinuityWorkerHistory(t, "first", true, hostResultHistory(), true, true)
}

func TestRealContinuityWorkerHostResultsQuotaFailoverAndRestart(t *testing.T) {
	executable := os.Getenv("COCKPIT_CONTINUITY_TEST_EXE")
	if executable == "" {
		t.Fatal("build the Rust host and set COCKPIT_CONTINUITY_TEST_EXE")
	}
	gin.SetMode(gin.TestMode)
	var mu sync.Mutex
	var bodies []map[string]any
	var headers []http.Header
	output := []any{map[string]any{"type": "reasoning", "id": "reasoning-a", "summary": []any{}, "encrypted_content": "original-cipher-a"}}
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
			w.WriteHeader(400)
			return
		}
		mu.Lock()
		bodies = append(bodies, body)
		headers = append(headers, r.Header.Clone())
		index := len(bodies)
		mu.Unlock()
		if index == 2 && r.Header.Get("Authorization") == "Bearer upstream-a" {
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusTooManyRequests)
			_, _ = w.Write([]byte(`{"error":{"type":"usage_limit_reached","code":"usage_limit_reached","message":"Synthetic quota exhausted"}}`))
			return
		}
		items := []any{}
		if index == 1 {
			items = output
		}
		event, _ := json.Marshal(map[string]any{"type": "response.completed", "response": map[string]any{"id": fmt.Sprintf("host-response-%d", index), "status": "completed", "output": items}})
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = fmt.Fprintf(w, "data: %s\n\n", event)
	}))
	defer upstream.Close()
	m := continuityManifestFixture(t)
	spec := &apiKeySpec{ID: "legacy", Key: "test-client-key", Enabled: true, AccountIDs: []string{"local-a", "local-b"}}
	m.apiKeyByValue = map[string]*apiKeySpec{spec.Key: spec}
	m.ModelIDs = []string{"gpt-5.4"}
	cfg := &config.Config{}
	cfg.Codex.OptimizeMultiAgentV2 = true
	cfg.Codex.StreamBootstrapBuffering = true
	tracker := newRequestUsageTracker()
	manager := coreauth.NewManager(nil, &accountSlotSelector{manifest: m, tracker: tracker, fallback: &orderedAuthSelector{order: []string{"sdk-a", "sdk-b"}}}, &authHook{manifest: m})
	manager.SetConfig(cfg)
	manager.SetRetryConfig(0, 0, 2)
	manager.RegisterExecutor(runtimeexecutor.NewCodexExecutor(cfg))
	for _, id := range []string{"a", "b"} {
		registry.GetGlobalRegistry().RegisterClient("sdk-"+id, "codex", []*registry.ModelInfo{{ID: "gpt-5.4"}})
		defer registry.GetGlobalRegistry().UnregisterClient("sdk-" + id)
		auth := &coreauth.Auth{ID: "sdk-" + id, Provider: "codex", Status: coreauth.StatusActive, Attributes: map[string]string{"account_id": "local-" + id, "base_url": upstream.URL}, Metadata: map[string]any{"access_token": "upstream-" + id, "type": "codex", "refresh_owner": "cockpit_token_authority"}}
		if _, err := manager.Register(context.Background(), auth); err != nil {
			t.Fatal(err)
		}
	}
	m.authManager = manager
	dir := t.TempDir()
	token := strings.Repeat("a", 64)
	configPath := filepath.Join(dir, "continuity.json")
	configData, _ := json.Marshal(map[string]any{"identity_path": filepath.Join(dir, "identity.sqlite"), "token": token, "owners": map[string]string{"local-a": "owner-a", "local-b": "owner-b"}, "caller_accounts": map[string][]string{"native:primary": {"local-a", "local-b"}}})
	if err := os.WriteFile(configPath, configData, 0600); err != nil {
		t.Fatal(err)
	}
	start := func() {
		t.Helper()
		var err error
		m.continuityClient, err = startContinuityWorker(context.Background(), &continuityServiceSpec{Version: 1, Executable: executable, ConfigPath: configPath, Token: token}, m)
		if err != nil {
			t.Fatal(err)
		}
	}
	start()
	defer func() { m.continuityClient.Close() }()
	router := (&relayServer{manifest: m, policy: &requestPolicy{manifest: m, tracker: tracker}, runtime: manager, authManager: manager, cfg: cfg}).router()
	send := func(input []any, previous, responseID string) {
		t.Helper()
		body := map[string]any{"model": "gpt-5.4", "stream": true, "input": input, "client_metadata": map[string]any{"session_id": "client-root", "thread_id": "client-root"}}
		if previous != "" {
			body["previous_response_id"] = previous
		}
		encoded, err := json.Marshal(body)
		if err != nil {
			t.Fatal(err)
		}
		req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(string(encoded)))
		req.Header.Set("Authorization", "Bearer test-client-key")
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("User-Agent", "Codex Desktop/1")
		w := httptest.NewRecorder()
		router.ServeHTTP(w, req)
		if w.Code != 200 || !strings.Contains(w.Body.String(), "response.completed") || !strings.Contains(w.Body.String(), responseID) {
			t.Fatalf("continuation failed: %d %s", w.Code, w.Body.String())
		}
	}
	initial := hostResultHistory()
	send(initial, "", "host-response-1")
	follow := append(append([]any{}, initial...), output...)
	follow = append(follow, map[string]any{"role": "user", "content": "continue the same task"})
	send(follow, "host-response-1", "host-response-3")
	m.continuityClient.Close()
	start()
	send([]any{map[string]any{"role": "user", "content": "continue after restart"}}, "host-response-3", "host-response-4")
	mu.Lock()
	defer mu.Unlock()
	if len(bodies) != 4 {
		t.Fatalf("expected A, A(429), B, B; got %d requests", len(bodies))
	}
	for i, expected := range []string{"a", "a", "b", "b"} {
		if headers[i].Get("Authorization") != "Bearer upstream-"+expected {
			t.Fatalf("attempt %d did not use %s", i, expected)
		}
	}
	if !reflect.DeepEqual(bodies[0]["input"], initial) || !reflect.DeepEqual(bodies[2]["input"], follow) {
		t.Fatal("host records, tool IDs or ciphertext changed during failover")
	}
	last := bodies[3]["input"].([]any)
	if len(last) < len(follow) || !reflect.DeepEqual(last[:len(follow)], follow) {
		t.Fatal("worker restart lost the complete history")
	}
	for _, index := range []int{2, 3} {
		if _, exists := bodies[index]["previous_response_id"]; exists {
			t.Fatal("stateless OAuth request retained an upstream response ID")
		}
	}
	if headers[0].Get("Thread-Id") == headers[2].Get("Thread-Id") || headers[2].Get("Thread-Id") != headers[3].Get("Thread-Id") {
		t.Fatal("account switch/restart did not preserve distinct conversation identities")
	}
}
