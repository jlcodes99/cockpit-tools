//go:build continuity_integration

package main

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
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

// Exercise the source boundary through the real executor and Rust store, rather
// than inserting already marked output or repairing any incoming history.
func TestRealContinuityWorkerPersistsPlaintextMarkersAndCipher(t *testing.T) {
	executable := os.Getenv("COCKPIT_CONTINUITY_TEST_EXE")
	if executable == "" {
		t.Fatal("build the Rust host and set COCKPIT_CONTINUITY_TEST_EXE")
	}
	sqlite, err := exec.LookPath("sqlite3")
	if err != nil {
		t.Fatal("sqlite3 is required to verify the real continuity database", err)
	}
	gin.SetMode(gin.TestMode)
	const cipher = "gAAAA-explicit-cipher-fixture-preserve-verbatim"
	messages := []string{"Review the new parser and report the result.", "Keep the original request IDs and arguments."}
	var output []map[string]any
	for i, name := range []string{"spawn_agent", "send_message", "followup_task"} {
		message := cipher
		if i < len(messages) {
			message = messages[i]
		}
		arguments, _ := json.Marshal(map[string]any{"message": message, "recipient": "/root/worker"})
		item := map[string]any{"id": fmt.Sprintf("marker-tool-%d", i), "type": "function_call", "status": "completed", "call_id": fmt.Sprintf("marker-call-%d", i), "namespace": "collaboration", "name": name, "arguments": string(arguments)}
		if i == 2 {
			item["encrypted_function_args"] = []any{cipher}
			item["encrypted_content"] = cipher
			item["signature"] = "explicit-wire-signature"
		}
		output = append(output, item)
	}
	var mu sync.Mutex
	var requests []map[string]any
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		mu.Lock()
		requests = append(requests, body)
		index := len(requests)
		mu.Unlock()
		w.Header().Set("Content-Type", "text/event-stream")
		items := []map[string]any{}
		if index == 1 {
			items = output
			for i, item := range items {
				for _, eventType := range []string{"response.output_item.added", "response.output_item.done"} {
					event, _ := json.Marshal(map[string]any{"type": eventType, "output_index": i, "item": item})
					_, _ = fmt.Fprintf(w, "data: %s\n\n", event)
				}
			}
		}
		event, _ := json.Marshal(map[string]any{"type": "response.completed", "response": map[string]any{"id": fmt.Sprintf("marker-response-%d", index), "status": "completed", "output": items}})
		_, _ = fmt.Fprintf(w, "data: %s\n\n", event)
	}))
	defer upstream.Close()

	m := continuityManifestFixture(t)
	spec := &apiKeySpec{ID: "legacy", Key: "marker-client-key", Enabled: true, AccountIDs: []string{"local-a"}}
	m.apiKeyByValue = map[string]*apiKeySpec{spec.Key: spec}
	m.ModelIDs = []string{"gpt-5.4"}
	cfg := &config.Config{}
	cfg.Codex.OptimizeMultiAgentV2 = true
	cfg.Codex.StreamBootstrapBuffering = true
	tracker := newRequestUsageTracker()
	manager := coreauth.NewManager(nil, &accountSlotSelector{manifest: m, tracker: tracker, fallback: &orderedAuthSelector{order: []string{"sdk-a"}}}, &authHook{manifest: m})
	manager.SetConfig(cfg)
	manager.SetRetryConfig(0, 0, 1)
	manager.RegisterExecutor(runtimeexecutor.NewCodexExecutor(cfg))
	registry.GetGlobalRegistry().RegisterClient("sdk-a", "codex", []*registry.ModelInfo{{ID: "gpt-5.4"}})
	defer registry.GetGlobalRegistry().UnregisterClient("sdk-a")
	if _, err := manager.Register(context.Background(), &coreauth.Auth{ID: "sdk-a", Provider: "codex", Status: coreauth.StatusActive, Attributes: map[string]string{"account_id": "local-a", "base_url": upstream.URL}, Metadata: map[string]any{"access_token": "local-oauth-fixture", "type": "codex", "refresh_owner": "cockpit_token_authority"}}); err != nil {
		t.Fatal(err)
	}
	m.authManager = manager
	dir := t.TempDir()
	database := filepath.Join(dir, "identity.sqlite")
	token := strings.Repeat("c", 64)
	configPath := filepath.Join(dir, "continuity.json")
	configData, _ := json.Marshal(map[string]any{"identity_path": database, "token": token, "owners": map[string]string{"local-a": "owner-a"}, "caller_accounts": map[string][]string{"native:primary": {"local-a"}}})
	if err := os.WriteFile(configPath, configData, 0600); err != nil {
		t.Fatal(err)
	}
	workerSpec := &continuityServiceSpec{Version: 1, Executable: executable, ConfigPath: configPath, Token: token}
	start := func() {
		t.Helper()
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
	send := func(body map[string]any) string {
		t.Helper()
		raw, _ := json.Marshal(body)
		req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(string(raw)))
		req.Header.Set("Authorization", "Bearer "+spec.Key)
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("User-Agent", "codex-tui/0.145.0")
		req.Header.Set("Session-Id", "marker-root")
		req.Header.Set("Thread-Id", "marker-root")
		w := httptest.NewRecorder()
		router.ServeHTTP(w, req)
		if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "response.completed") {
			t.Fatalf("response failed: %d %s", w.Code, w.Body.String())
		}
		return w.Body.String()
	}
	query := func(sql string) []byte {
		t.Helper()
		data, err := exec.Command(sqlite, "-readonly", database, sql).CombinedOutput()
		if err != nil {
			t.Fatalf("query continuity database: %v: %s", err, data)
		}
		return data
	}
	assertItems := func(items []map[string]any) {
		t.Helper()
		for i, original := range output {
			var actual map[string]any
			for _, item := range items {
				if item["id"] == original["id"] {
					actual = item
				}
			}
			expected := make(map[string]any, len(original)+1)
			for key, value := range original {
				expected[key] = value
			}
			if i < 2 {
				expected["encrypted_function_args"] = []any{}
			}
			if !reflect.DeepEqual(actual, expected) {
				t.Fatalf("new output did not retain its identity, arguments and encryption evidence: got %#v want %#v", actual, expected)
			}
		}
	}
	var definitions []map[string]any
	for i, item := range output {
		message := map[string]any{"type": "string", "encrypted": i == 2}
		definitions = append(definitions, map[string]any{"type": "function", "name": item["name"], "parameters": map[string]any{"type": "object", "properties": map[string]any{"message": message}}})
	}
	input := []any{map[string]any{"type": "additional_tools", "role": "developer", "tools": []any{map[string]any{"type": "namespace", "name": "collaboration", "tools": definitions}}}, map[string]any{"role": "user", "content": "Start the new collaboration calls."}}
	send(map[string]any{"model": "gpt-5.4", "stream": true, "input": input})
	var checkpoint []map[string]any
	if err := json.Unmarshal(query("SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='native:primary' AND response='marker-response-1';"), &checkpoint); err != nil {
		t.Fatal("new response checkpoint was not saved", err)
	}
	assertItems(checkpoint)
	for _, message := range messages {
		digest := fmt.Sprintf("%x", sha256.Sum256([]byte(message)))
		if count := strings.TrimSpace(string(query("SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted' AND digest='" + digest + "';"))); count != "0" {
			t.Fatalf("plaintext message was registered as encrypted: %s", count)
		}
	}
	digest := fmt.Sprintf("%x", sha256.Sum256([]byte(cipher)))
	if count := strings.TrimSpace(string(query("SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted' AND digest='" + digest + "';"))); count != "1" {
		t.Fatalf("explicit ciphertext ownership was not preserved: %s", count)
	}

	m.continuityClient.Close()
	start()
	var continuation []any
	for _, item := range output {
		continuation = append(continuation, map[string]any{"type": "function_call_output", "call_id": item["call_id"], "output": "completed"})
	}
	continuation = append(continuation, map[string]any{"role": "user", "content": "Continue after the worker restart."})
	send(map[string]any{"model": "gpt-5.4", "stream": true, "previous_response_id": "marker-response-1", "input": continuation})
	mu.Lock()
	defer mu.Unlock()
	if len(requests) != 2 {
		t.Fatalf("unexpected upstream request count: %d", len(requests))
	}
	// Canonical bind restores the original additional_tools block. The marker
	// policy must follow these final mixed declarations, not its earlier rename.
	firstInput, _ := requests[0]["input"].([]any)
	if len(firstInput) == 0 {
		t.Fatal("missing final canonical input")
	}
	canonical := firstInput[0].(map[string]any)["tools"].([]any)[0].(map[string]any)
	if canonical["name"] != "collaboration" {
		t.Fatalf("fixture did not exercise canonical namespace restoration: %#v", canonical)
	}
	for i, definition := range canonical["tools"].([]any) {
		properties := definition.(map[string]any)["parameters"].(map[string]any)["properties"].(map[string]any)
		if encrypted := properties["message"].(map[string]any)["encrypted"]; encrypted != (i == 2) {
			t.Fatalf("fixture did not restore the final mixed encryption declarations: %#v", canonical)
		}
	}
	var replay []map[string]any
	for _, item := range requests[1]["input"].([]any) {
		replay = append(replay, item.(map[string]any))
	}
	assertItems(replay)
}
