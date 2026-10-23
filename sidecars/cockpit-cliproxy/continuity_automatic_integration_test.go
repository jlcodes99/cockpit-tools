//go:build continuity_integration

package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	runtimeexecutor "github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/tidwall/gjson"
)

func TestRealContinuityWorkerAdoptsUnknownHistoryAcrossGoRestartAndAccounts(t *testing.T) {
	executable := os.Getenv("COCKPIT_CONTINUITY_TEST_EXE")
	if executable == "" {
		t.Fatal("build the Rust host and set COCKPIT_CONTINUITY_TEST_EXE")
	}
	gin.SetMode(gin.TestMode)
	var mu sync.Mutex
	var bodies [][]byte
	var headers []http.Header
	var failNext atomic.Bool
	failNext.Store(true)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Error(err)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		mu.Lock()
		bodies = append(bodies, body)
		headers = append(headers, r.Header.Clone())
		index := len(bodies)
		mu.Unlock()
		if failNext.Swap(false) {
			w.WriteHeader(http.StatusBadRequest)
			_, _ = io.WriteString(w, `{"error":{"code":"invalid_encrypted_content","message":"synthetic upstream history rejection"}}`)
			return
		}
		w.Header().Set("Content-Type", "text/event-stream")
		output := `[]`
		if gjson.GetBytes(body, "tool_choice.type").String() == "image_generation" {
			output = `[{"type":"image_generation_call","id":"imported-history-image","status":"completed","result":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]`
		}
		_, _ = fmt.Fprintf(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"automatic-response-%d\",\"status\":\"completed\",\"output\":%s}}\n\n", index, output)
	}))
	defer upstream.Close()
	dir := t.TempDir()
	configPath := filepath.Join(dir, "continuity.json")
	token := strings.Repeat("c", 64)
	configData, err := json.Marshal(map[string]any{
		"identity_path": filepath.Join(dir, "identity.sqlite"), "token": token,
		"owners":          map[string]string{"local-a": "owner-a", "local-b": "owner-b"},
		"caller_accounts": map[string][]string{"native:primary": {"local-a", "local-b"}, "secondary": {"local-b"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(configPath, configData, 0600); err != nil {
		t.Fatal(err)
	}
	goTestExecutable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: 10 * time.Second}
	defer client.CloseIdleConnections()
	var gatewayURL string
	var stopGateway func()
	// A fresh Go process owns each worker. Only the shared SQLite file survives.
	restart := func(ids ...string) {
		t.Helper()
		if stopGateway != nil {
			stopGateway()
		}
		cmd := exec.Command(goTestExecutable, "-test.run=^TestContinuityAutomaticGatewaySubprocess$", "-test.timeout=45s")
		configureContinuityProcess(cmd)
		cmd.Env = append(os.Environ(), "COCKPIT_CONTINUITY_GATEWAY_TEST_CONFIG="+configPath, "COCKPIT_CONTINUITY_GATEWAY_TEST_UPSTREAM="+upstream.URL, "COCKPIT_CONTINUITY_GATEWAY_TEST_AUTH_IDS="+strings.Join(ids, ","))
		stdin, err := cmd.StdinPipe()
		if err != nil {
			t.Fatal(err)
		}
		stdout, err := cmd.StdoutPipe()
		if err != nil {
			_ = stdin.Close()
			t.Fatal(err)
		}
		cmd.Stderr = io.Discard
		if err := cmd.Start(); err != nil {
			_ = stdin.Close()
			_ = stdout.Close()
			t.Fatal(err)
		}
		finished := make(chan error, 1)
		go func() { finished <- cmd.Wait(); close(finished) }()
		var once sync.Once
		stopGateway = func() {
			once.Do(func() {
				_ = stdin.Close()
				select {
				case <-finished:
				case <-time.After(5 * time.Second):
					_ = cmd.Process.Kill()
					select {
					case <-finished:
					case <-time.After(time.Second):
					}
				}
			})
		}
		ready := make(chan string, 1)
		go func() {
			scanner := bufio.NewScanner(stdout)
			for scanner.Scan() {
				if endpoint, ok := strings.CutPrefix(scanner.Text(), "automatic-gateway-ready "); ok {
					ready <- endpoint
				}
			}
		}()
		select {
		case gatewayURL = <-ready:
		case err := <-finished:
			t.Fatalf("automatic Go gateway exited before startup: %v", err)
		case <-time.After(12 * time.Second):
			stopGateway()
			t.Fatal("automatic Go gateway startup timed out")
		}
	}
	t.Cleanup(func() {
		if stopGateway != nil {
			stopGateway()
		}
	})
	restart("a", "b")
	history := json.RawMessage(`[{"role":"user","content":"historical question"},{"type":"reasoning","id":"old-reasoning","summary":[],"encrypted_content":"old-reasoning-cipher"},{"type":"compaction","id":"old-compaction","encrypted_content":"old-compaction-cipher"},{"type":"function_call","id":"old-tool","call_id":"old-call","name":"spawn_agent","namespace":"collaboration","arguments":"{\"message\":\"old-agent-cipher\"}"},{"type":"function_call_output","call_id":"old-call","output":[{"type":"encrypted_content","encrypted_content":"old-tool-cipher"}]},{"role":"user","content":"continue the old conversation"}]`)
	requestBody := func(input json.RawMessage, thread, previous string) string {
		t.Helper()
		body := map[string]any{"model": "gpt-5.4", "stream": true, "input": input, "client_metadata": map[string]any{"session_id": "old-root", "thread_id": thread}}
		if thread != "old-root" {
			body["client_metadata"].(map[string]any)["x-codex-parent-thread-id"] = "old-root"
		}
		if previous != "" {
			body["previous_response_id"] = previous
		}
		encoded, err := json.Marshal(body)
		if err != nil {
			t.Fatal(err)
		}
		return string(encoded)
	}
	send := func(body, thread string, expected int, keys ...string) {
		t.Helper()
		request, err := http.NewRequest(http.MethodPost, gatewayURL+"/v1/responses", strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		key := "automatic-client"
		if len(keys) > 0 {
			key = keys[0]
		}
		request.Header.Set("Authorization", "Bearer "+key)
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Session-Id", "old-root")
		request.Header.Set("Thread-Id", thread)
		response, err := client.Do(request)
		if err != nil {
			t.Fatal(err)
		}
		actual, err := io.ReadAll(response.Body)
		_ = response.Body.Close()
		if err != nil || response.StatusCode != expected || expected == http.StatusOK && !strings.Contains(string(actual), "response.completed") {
			t.Fatalf("automatic continuation: status=%d want=%d body=%s err=%v", response.StatusCode, expected, actual, err)
		}
	}
	initial := requestBody(history, "old-root", "")
	send(initial, "old-root", http.StatusBadRequest)
	// A failed import must not publish input ownership. Another authorized caller
	// can still submit that previously unregistered history to its own account.
	failNext.Store(true)
	send(initial, "old-root", http.StatusBadRequest, "secondary-client")
	send(initial, "old-root", http.StatusOK)
	// Once success records ownership, the other caller cannot claim that history.
	send(initial, "old-root", http.StatusConflict, "secondary-client")
	child := json.RawMessage(`[{"role":"user","content":"continue the child task"},{"type":"agent_message","author":"/root","recipient":"/root/worker","content":[{"type":"encrypted_content","encrypted_content":"old-agent-cipher"}]},{"type":"reasoning","id":"child-old-id","encrypted_content":"old-child-cipher","summary":[]}]`)
	send(requestBody(child, "old-child", ""), "old-child", http.StatusOK)
	restart("a", "b")
	send(requestBody(json.RawMessage(`[{"role":"user","content":"after Go restart"}]`), "old-root", "automatic-response-3"), "old-root", http.StatusOK)
	removed, err := client.Post(gatewayURL+"/_continuity-test/remove-a", "application/json", nil)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = io.Copy(io.Discard, removed.Body)
	_ = removed.Body.Close()
	if removed.StatusCode != http.StatusNoContent {
		t.Fatal("fixture account removal failed", removed.StatusCode)
	}
	fullHistory := json.RawMessage(strings.TrimSuffix(string(history), "]") + `,{"role":"user","content":"after Go restart"}]`)
	follow := requestBody(fullHistory, "old-root", "automatic-response-5")
	send(follow, "old-root", http.StatusOK)
	failNext.Store(true)
	send(follow, "old-root", http.StatusBadRequest)
	restart("b")
	send(requestBody(json.RawMessage(`[{"role":"user","content":"resume last success after failed attempt"}]`), "old-root", "automatic-response-6"), "old-root", http.StatusOK)
	imageHistory := json.RawMessage(`[{"role":"user","content":"old visual context"},{"type":"reasoning","id":"old-image-item","summary":[],"encrypted_content":"unknown-image-history-cipher"},{"role":"user","content":"draw using the old context"}]`)
	imageBody := strings.TrimSuffix(requestBody(imageHistory, "old-image-child", ""), "}") + `,"tools":[{"type":"image_generation","model":"gpt-image-2.5"}],"tool_choice":{"type":"image_generation"}}`
	send(imageBody, "old-image-child", http.StatusOK)
	mu.Lock()
	defer mu.Unlock()
	if len(bodies) != 9 {
		t.Fatalf("unexpected replay count: %d", len(bodies))
	}
	for i, actual := range bodies {
		account := "Bearer upstream-a"
		if i == 1 || i >= 5 {
			account = "Bearer upstream-b"
		}
		if headers[i].Get("Authorization") != account {
			t.Errorf("attempt %d used wrong account: %s", i, headers[i].Get("Authorization"))
		}
		if i == 0 || i == 1 || i == 2 || i == 5 || i == 6 {
			for _, value := range []string{"old-reasoning-cipher", "old-compaction-cipher", "old-agent-cipher", "old-tool-cipher", "old-reasoning", "old-tool", "old-call"} {
				if !strings.Contains(string(actual), value) {
					t.Errorf("attempt %d lost original history %q: %s", i, value, actual)
				}
			}
		}
	}
	if headers[2].Get("Session-Id") != headers[3].Get("Session-Id") || headers[2].Get("Thread-Id") == headers[3].Get("Thread-Id") {
		t.Fatal("parent/child identities were merged or split across sessions")
	}
	if headers[2].Get("Session-Id") != headers[4].Get("Session-Id") || headers[2].Get("Thread-Id") != headers[4].Get("Thread-Id") {
		t.Fatal("Go restart lost the successful imported history mapping")
	}
	if headers[4].Get("Session-Id") == headers[5].Get("Session-Id") || headers[4].Get("Thread-Id") == headers[5].Get("Thread-Id") || gjson.GetBytes(bodies[5], "previous_response_id").Exists() {
		t.Fatal("cross-account replay reused A's private identifiers")
	}
	if headers[5].Get("Session-Id") != headers[7].Get("Session-Id") || headers[5].Get("Thread-Id") != headers[7].Get("Thread-Id") || gjson.GetBytes(bodies[7], "previous_response_id").String() != "automatic-response-6" {
		t.Fatal("failed continuation or Go restart replaced the last successful B state")
	}
	if !strings.Contains(string(bodies[8]), "unknown-image-history-cipher") || !strings.Contains(string(bodies[8]), "old-image-item") || gjson.GetBytes(bodies[8], "tool_choice.type").String() != "image_generation" {
		t.Fatalf("OAuth image pool lost Responses conversation history: %s", bodies[8])
	}
}

func TestContinuityAutomaticGatewaySubprocess(t *testing.T) {
	configPath := os.Getenv("COCKPIT_CONTINUITY_GATEWAY_TEST_CONFIG")
	if configPath == "" {
		return
	}
	gin.SetMode(gin.TestMode)
	m := continuityManifestFixture(t)
	key := &apiKeySpec{ID: "legacy", Key: "automatic-client", Enabled: true, AccountIDs: []string{"local-a", "local-b"}, ImageGenerationAccountIDs: []string{"local-a", "local-b", "images-api"}}
	secondary := &apiKeySpec{ID: "secondary", Key: "secondary-client", Enabled: true, AccountIDs: []string{"local-b"}}
	m.apiKeyByValue = map[string]*apiKeySpec{key.Key: key, secondary.Key: secondary}
	m.ModelIDs = []string{"gpt-5.4"}
	m.ImageGenerationModel = defaultImagesToolModel
	m.ImageGenerationMainModel = "gpt-5.4"
	for i := range m.Accounts {
		m.Accounts[i].AuthKind = "oauth"
		m.Accounts[i].PlanType = "plus"
	}
	// A disabled external source forces the mixed-pool dispatch branch while
	// selection still uses an eligible OAuth account and the complete Responses.
	m.Accounts = append(m.Accounts, accountSpec{ID: "images-api", AuthKind: "api_key", ImageGenerationPolicy: "disabled", ImageGenerationUpstream: &imageAPIUpstreamSpec{BaseURL: os.Getenv("COCKPIT_CONTINUITY_GATEWAY_TEST_UPSTREAM"), APIKey: "unused-fixture-key", ModelIDs: []string{defaultImagesToolModel}}})
	for i := range m.Accounts {
		m.accountByID[m.Accounts[i].ID] = &m.Accounts[i]
	}
	cfg := &config.Config{}
	cfg.Codex.StreamBootstrapBuffering = true
	cfg.Codex.IdentityConfuse = true
	tracker := newRequestUsageTracker()
	selector := buildCoreAuthSelectorWithConcurrency(cfg, &orderedAuthSelector{order: []string{"sdk-a", "sdk-b"}}, m, nil, tracker)
	manager := coreauth.NewManager(nil, selector, &authHook{manifest: m})
	manager.SetConfig(cfg)
	manager.SetRetryConfig(0, 0, 2)
	manager.RegisterExecutor(runtimeexecutor.NewCodexExecutor(cfg))
	for _, id := range strings.Split(os.Getenv("COCKPIT_CONTINUITY_GATEWAY_TEST_AUTH_IDS"), ",") {
		registry.GetGlobalRegistry().RegisterClient("sdk-"+id, "codex", []*registry.ModelInfo{{ID: "gpt-5.4"}})
		defer registry.GetGlobalRegistry().UnregisterClient("sdk-" + id)
		if _, err := manager.Register(context.Background(), &coreauth.Auth{ID: "sdk-" + id, Provider: "codex", Status: coreauth.StatusActive, Attributes: map[string]string{"account_id": "local-" + id, "api_key": "upstream-" + id, "base_url": os.Getenv("COCKPIT_CONTINUITY_GATEWAY_TEST_UPSTREAM")}}); err != nil {
			t.Fatal(err)
		}
	}
	m.authManager = manager
	var err error
	m.continuityClient, err = startContinuityWorker(context.Background(), &continuityServiceSpec{Version: 1, Executable: os.Getenv("COCKPIT_CONTINUITY_TEST_EXE"), ConfigPath: configPath, Token: strings.Repeat("c", 64)}, m)
	if err != nil {
		t.Fatal("automatic worker startup", err)
	}
	defer m.continuityClient.Close()
	policy := &requestPolicy{manifest: m, tracker: tracker}
	router := (&relayServer{manifest: m, policy: policy, runtime: manager, authManager: manager, cfg: cfg}).router()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/_continuity-test/remove-a" {
			manager.Remove(r.Context(), "sdk-a")
			w.WriteHeader(http.StatusNoContent)
			return
		}
		router.ServeHTTP(w, r)
	}))
	defer server.Close()
	fmt.Println("automatic-gateway-ready " + server.URL)
	_, _ = io.Copy(io.Discard, os.Stdin)
}
