//go:build continuity_integration

package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	runtimeexecutor "github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

// Run with COCKPIT_CONTINUITY_TEST_EXE pointing at a freshly built host binary.
// All upstream traffic and credentials below are local fixtures, never accounts.
func TestRealContinuityWorkerCrossAccountAndRestart(t *testing.T) {
	runRealContinuityWorkerCrossAccountAndRestart(t, "first", false)
}

func TestRealContinuityWorkerLargeHistoryCrossAccountAndRestart(t *testing.T) {
	// Exercise the actual Go SDK executor and Rust worker, including a pure-ID
	// follow-up after restart. The accepted history exceeds the old hot cache.
	runRealContinuityWorkerCrossAccountAndRestart(t, strings.Repeat("x", 17*1024*1024), true)
}

func TestRealContinuityWorkerStreamedCheckpointCrossAccountAndRestart(t *testing.T) {
	runRealContinuityWorkerCrossAccountAndRestart(t, "first", true, true)
}

func TestRealContinuityWorkerLargeStreamedCheckpointCrossAccountAndRestart(t *testing.T) {
	runRealContinuityWorkerCrossAccountAndRestart(t, strings.Repeat("x", 17*1024*1024), true, true)
}

func runRealContinuityWorkerCrossAccountAndRestart(t *testing.T, initialText string, oauth bool, streamedCheckpoint ...bool) {
	runRealContinuityWorkerHistory(t, initialText, oauth, nil, false, streamedCheckpoint...)
}

func runRealContinuityWorkerHistory(t *testing.T, initialText string, oauth bool, legacy []any, optimize bool, streamedCheckpoint ...bool) {
	t.Helper()
	sparseTerminal := len(streamedCheckpoint) > 0 && streamedCheckpoint[0]
	executable := os.Getenv("COCKPIT_CONTINUITY_TEST_EXE")
	if executable == "" {
		t.Fatal("build the Rust host and set COCKPIT_CONTINUITY_TEST_EXE")
	}
	gin.SetMode(gin.TestMode)
	var mu sync.Mutex
	var seenHeaders []http.Header
	var seenBodies []map[string]any
	output := json.RawMessage(`[{"id":"reasoning-a","type":"reasoning","summary":[],"encrypted_content":"original-cipher-a"},{"id":"tool-a","type":"function_call","call_id":"call-a","name":"spawn_agent","namespace":"collaboration","arguments":"{\"message\":\"agent-message-a\"}"}]`)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
			w.WriteHeader(400)
			return
		}
		mu.Lock()
		seenHeaders = append(seenHeaders, r.Header.Clone())
		seenBodies = append(seenBodies, body)
		index := len(seenBodies)
		mu.Unlock()
		w.Header().Set("Content-Type", "text/event-stream")
		resultOutput := json.RawMessage(`[]`)
		if index == 1 {
			resultOutput = output
			if sparseTerminal {
				// Official HTTP responses may send complete items only through
				// output_item.done and leave the successful terminal output empty.
				var items []json.RawMessage
				if err := json.Unmarshal(output, &items); err != nil {
					t.Error(err)
					return
				}
				for i, item := range items {
					_, _ = fmt.Fprintf(w, "data: {\"type\":\"response.output_item.done\",\"output_index\":%d,\"item\":%s}\n\n", i, item)
				}
				resultOutput = json.RawMessage(`[]`)
			}
		}
		_, _ = fmt.Fprintf(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-%d\",\"status\":\"completed\",\"output\":%s}}\n\n", index, resultOutput)
	}))
	defer upstream.Close()
	m := continuityManifestFixture(t)
	spec := &apiKeySpec{ID: "legacy", Key: "test-client-key", Enabled: true, AccountIDs: []string{"local-a", "local-b"}}
	m.apiKeyByValue = map[string]*apiKeySpec{spec.Key: spec}
	m.ModelIDs = []string{"gpt-5.4"}
	cfg := &config.Config{}
	cfg.Codex.OptimizeMultiAgentV2 = optimize
	cfg.Codex.StreamBootstrapBuffering = true
	tracker := newRequestUsageTracker()
	manager := coreauth.NewManager(nil, &accountSlotSelector{manifest: m, tracker: tracker, fallback: &orderedAuthSelector{order: []string{"sdk-a", "sdk-b"}}}, &authHook{manifest: m})
	manager.SetConfig(cfg)
	manager.SetRetryConfig(0, 0, 2)
	manager.RegisterExecutor(runtimeexecutor.NewCodexExecutor(cfg))
	for _, id := range []string{"a", "b"} {
		registry.GetGlobalRegistry().RegisterClient("sdk-"+id, "codex", []*registry.ModelInfo{{ID: "gpt-5.4"}})
		defer registry.GetGlobalRegistry().UnregisterClient("sdk-" + id)
		auth := &coreauth.Auth{ID: "sdk-" + id, Provider: "codex", Status: coreauth.StatusActive, Attributes: map[string]string{"account_id": "local-" + id, "base_url": upstream.URL}}
		if oauth {
			auth.Metadata = map[string]any{"access_token": "upstream-" + id, "type": "codex", "refresh_owner": "cockpit_token_authority"}
		} else {
			auth.Attributes["api_key"] = "upstream-" + id
		}
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
	workerSpec := &continuityServiceSpec{Version: 1, Executable: executable, ConfigPath: configPath, Token: token}
	start := func() {
		var err error
		m.continuityClient, err = startContinuityWorker(context.Background(), workerSpec, m)
		if err != nil {
			t.Fatal("headless worker startup", err)
		}
	}
	start()
	defer func() { m.continuityClient.Close() }()
	policy := &requestPolicy{manifest: m, tracker: tracker}
	router := (&relayServer{manifest: m, policy: policy, runtime: manager, authManager: manager, cfg: cfg}).router()
	send := func(body string) {
		t.Helper()
		req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(body))
		req.Header.Set("Authorization", "Bearer test-client-key")
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("User-Agent", "codex-tui/0.145.0")
		req.Header.Set("Session-Id", "client-root")
		req.Header.Set("Thread-Id", "client-root")
		w := httptest.NewRecorder()
		router.ServeHTTP(w, req)
		if w.Code != 200 || !strings.Contains(w.Body.String(), "response.completed") {
			t.Fatalf("continuation failed: %d %s", w.Code, w.Body.String())
		}
	}
	initialMessage, _ := json.Marshal(map[string]any{"role": "user", "content": initialText})
	var firstMessage any
	if err := json.Unmarshal(initialMessage, &firstMessage); err != nil {
		t.Fatal(err)
	}
	initialInput := append([]any{firstMessage}, legacy...)
	initialJSON, err := json.Marshal(initialInput)
	if err != nil {
		t.Fatal(err)
	}
	initial := fmt.Sprintf(`{"model":"gpt-5.4","stream":true,"input":%s,"client_metadata":{"session_id":"client-root","thread_id":"client-root"}}`, initialJSON)
	send(initial)
	send(`{"model":"gpt-5.4","stream":true,"input":[{"type":"agent_message","author":"/root","recipient":"/root/worker","content":[{"type":"encrypted_content","encrypted_content":"agent-message-a"}]}],"client_metadata":{"session_id":"client-root","thread_id":"client-child","x-codex-parent-thread-id":"client-root"}}`)
	manager.Remove(context.Background(), "sdk-a")
	follow := fmt.Sprintf(`{"model":"gpt-5.4","stream":true,"previous_response_id":"response-1","input":[%s,%s,{"type":"function_call_output","call_id":"call-a","output":"agent finished"},{"role":"user","content":"continue"}],"client_metadata":{"session_id":"client-root","thread_id":"client-root"}}`, strings.TrimSuffix(strings.TrimPrefix(string(initialJSON), "["), "]"), strings.TrimSuffix(strings.TrimPrefix(string(output), "["), "]"))
	if sparseTerminal {
		follow = `{"model":"gpt-5.4","stream":true,"previous_response_id":"response-1","input":[{"type":"function_call_output","call_id":"call-a","output":"agent finished"},{"role":"user","content":"continue"}],"client_metadata":{"session_id":"client-root","thread_id":"client-root"}}`
	}
	send(follow)
	m.continuityClient.Close()
	start()
	send(`{"model":"gpt-5.4","stream":true,"previous_response_id":"response-3","input":[{"role":"user","content":"after worker restart"}],"client_metadata":{"session_id":"client-root","thread_id":"client-root"}}`)
	mu.Lock()
	defer mu.Unlock()
	if len(seenBodies) != 4 {
		t.Fatal("unexpected upstream count", len(seenBodies))
	}
	for i, headers := range seenHeaders {
		expected := "Bearer upstream-b"
		if i < 2 {
			expected = "Bearer upstream-a"
		}
		if headers.Get("Authorization") != expected {
			t.Fatal("wrong real upstream account", i, headers.Get("Authorization"))
		}
		if headers.Get("Session-Id") == "client-root" || headers.Get("Thread-Id") == "client-root" {
			t.Fatal("client identity leaked without mapping", i)
		}
	}
	if seenHeaders[0].Get("Session-Id") != seenHeaders[1].Get("Session-Id") || seenHeaders[0].Get("Thread-Id") == seenHeaders[1].Get("Thread-Id") {
		t.Fatal("child agent identity did not remain distinct within the parent session")
	}
	if seenHeaders[0].Get("Session-Id") == seenHeaders[2].Get("Session-Id") || seenHeaders[0].Get("Thread-Id") == seenHeaders[2].Get("Thread-Id") {
		t.Fatal("cross-account identities reused")
	}
	if seenHeaders[2].Get("Session-Id") != seenHeaders[3].Get("Session-Id") || seenHeaders[2].Get("Thread-Id") != seenHeaders[3].Get("Thread-Id") {
		t.Fatal("restart lost B identity")
	}
	if _, exists := seenBodies[2]["previous_response_id"]; exists {
		t.Fatal("A previous_response_id forwarded to B")
	}
	second, _ := json.Marshal(seenBodies[2])
	if !strings.Contains(string(second), "original-cipher-a") || !strings.Contains(string(second), "agent-message-a") || !strings.Contains(string(second), "agent finished") {
		t.Fatal("full cipher/tool history was altered", string(second))
	}
	if oauth {
		firstInput, lastInput := seenBodies[0]["input"].([]any), seenBodies[3]["input"].([]any)
		if len(lastInput) < len(firstInput) || !reflect.DeepEqual(firstInput, initialInput) || !reflect.DeepEqual(firstInput, lastInput[:len(firstInput)]) {
			t.Fatal("restart did not preserve the complete original input")
		}
		if _, exists := seenBodies[3]["previous_response_id"]; exists {
			t.Fatal("OAuth HTTP continuation did not expand its durable history")
		}
	}
	child := seenBodies[1]["input"].([]any)[0].(map[string]any)
	if child["type"] != "agent_message" || child["content"].([]any)[0].(map[string]any)["type"] != "encrypted_content" {
		t.Fatal("official child agent ciphertext was reinterpreted as plaintext", child)
	}
	// Explicit Close ends the headless worker without waiting for a GUI shutdown.
	endpoint := m.continuityClient.endpoint
	m.continuityClient.Close()
	check := &http.Client{Timeout: time.Second}
	if response, err := check.Get(endpoint); err == nil {
		_, _ = io.Copy(io.Discard, response.Body)
		_ = response.Body.Close()
		t.Fatal("worker listener survived parent Close")
	}
}
