//go:build continuity_integration

package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// Exercise the actual parent pipe, Go process and Rust worker together, without
// real credentials or external services. The replacement uses the same port/DB.
func TestRealGatewayDrainKeepsWorkerAndLateRecords(t *testing.T) {
	host, binary := os.Getenv("COCKPIT_CONTINUITY_TEST_EXE"), os.Getenv("COCKPIT_GATEWAY_TEST_EXE")
	if host == "" || binary == "" {
		t.Fatal("set freshly built COCKPIT_CONTINUITY_TEST_EXE and COCKPIT_GATEWAY_TEST_EXE")
	}
	release := make(chan struct{})
	var once sync.Once
	var mu sync.Mutex
	var upstreamHeaders []http.Header
	var upstreamBodies []string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		data, _ := io.ReadAll(r.Body)
		mu.Lock()
		upstreamHeaders = append(upstreamHeaders, r.Header.Clone())
		upstreamBodies = append(upstreamBodies, string(data))
		mu.Unlock()
		id := "b"
		if r.Header.Get("Authorization") == "Bearer upstream-a" {
			id = "a"
		}
		w.Header().Set("Content-Type", "text/event-stream")
		fmt.Fprintf(w, "data: {\"type\":\"response.created\",\"response\":{\"id\":\"response-%s\"}}\n\n", id)
		fmt.Fprintf(w, "data: {\"type\":\"response.output_text.delta\",\"delta\":\"%s\"}\n\n", id)
		w.(http.Flusher).Flush()
		if id == "a" {
			<-release
		}
		fmt.Fprintf(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-%s\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":4,\"output_tokens\":1,\"total_tokens\":5}}}\n\n", id)
	}))
	defer func() { once.Do(func() { close(release) }); upstream.Close() }()
	dir := t.TempDir()
	token := strings.Repeat("d", 64)
	wc, _ := json.Marshal(map[string]any{"identity_path": filepath.Join(dir, "identity.sqlite"), "token": token, "owners": map[string]string{"local-a": "owner-a", "local-b": "owner-b"}, "caller_accounts": map[string][]string{"native:primary": {"local-a", "local-b"}}})
	workerConfig := filepath.Join(dir, "continuity.json")
	if err := os.WriteFile(workerConfig, wc, 0600); err != nil {
		t.Fatal(err)
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := listener.Addr().(*net.TCPAddr).Port
	listener.Close()
	type process struct {
		stdin  io.WriteCloser
		done   chan error
		exited chan struct{}
		mu     sync.Mutex
		events []map[string]any
	}
	start := func(id string) *process {
		t.Helper()
		sub := filepath.Join(dir, id)
		os.Mkdir(sub, 0700)
		m := &manifest{Continuity: &continuityServiceSpec{Version: 1, Executable: host, ConfigPath: workerConfig, Token: token},
			ModelIDs: []string{"gpt-5.4"}, DiagnosticsControlKey: "host-control", APIKeys: []apiKeySpec{{ID: "legacy", Key: "client-key", Enabled: true, AccountIDs: []string{"local-" + id}}},
			Accounts: []accountSpec{{ID: "local-" + id, AuthKind: "api_key", AuthID: id + ".json", ContinuityOwnerID: "owner-" + id, UpstreamAPIKey: "upstream-" + id, UpstreamBaseURL: upstream.URL, GatewayIdentity: gatewayIdentityFixture()}}}
		if id == "b" {
			m.Accounts[0].GatewayIdentity = json.RawMessage(strings.ReplaceAll(strings.ReplaceAll(string(gatewayIdentityFixture()), strings.Repeat("a", 64), strings.Repeat("b", 64)), "11111111-1111-4111-8111-111111111111", "22222222-2222-4222-8222-222222222222"))
		}
		raw, _ := json.Marshal(m)
		manifestPath := filepath.Join(sub, "manifest.json")
		os.WriteFile(manifestPath, raw, 0600)
		cfg := fmt.Sprintf("host: 127.0.0.1\nport: %d\nauth-dir: %q\napi-keys: [client-key]\ncodex-api-key:\n  - api-key: upstream-%s\n    base-url: %q\n    models:\n      - name: gpt-5.4\n", port, filepath.Join(sub, "auth"), id, upstream.URL)
		configPath := filepath.Join(sub, "config.yaml")
		os.WriteFile(configPath, []byte(cfg), 0600)
		cmd := exec.Command(binary, "--config", configPath, "--manifest", manifestPath, "--control-stdin")
		p := &process{done: make(chan error, 1), exited: make(chan struct{})}
		p.stdin, err = cmd.StdinPipe()
		if err != nil {
			t.Fatal(err)
		}
		stdout, _ := cmd.StdoutPipe()
		cmd.Stderr = io.Discard
		ready := make(chan struct{}, 1)
		if err = cmd.Start(); err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() {
			p.stdin.Close()
			select {
			case <-p.exited:
			default:
				_ = cmd.Process.Kill()
				<-p.exited
			}
		})
		go func() {
			scan := bufio.NewScanner(stdout)
			for scan.Scan() {
				var event map[string]any
				if json.Unmarshal(scan.Bytes(), &event) == nil {
					p.mu.Lock()
					p.events = append(p.events, event)
					p.mu.Unlock()
					if event["type"] == "ready" {
						ready <- struct{}{}
					}
				}
			}
			waitErr := cmd.Wait()
			close(p.exited)
			p.done <- waitErr
		}()
		select {
		case <-ready:
		case err = <-p.done:
			t.Fatalf("gateway %s startup failed: %v", id, err)
		case <-time.After(15 * time.Second):
			t.Fatal("gateway ready timeout")
		}
		return p
	}
	client := &http.Client{Timeout: 15 * time.Second}
	send := func(body string) *http.Response {
		t.Helper()
		req, _ := http.NewRequest("POST", fmt.Sprintf("http://127.0.0.1:%d/v1/responses", port), strings.NewReader(body))
		req.Header.Set("Authorization", "Bearer client-key")
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Session-Id", "client-root")
		req.Header.Set("Thread-Id", "client-root")
		response, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		if response.StatusCode != 200 {
			data, _ := io.ReadAll(response.Body)
			response.Body.Close()
			t.Fatalf("gateway failure: %d %s", response.StatusCode, data)
		}
		return response
	}
	a := start("a")
	first := send(`{"model":"gpt-5.4","stream":true,"input":[{"role":"user","content":"remember history"},{"type":"reasoning","encrypted_content":"original-cipher"}]}`)
	reader := bufio.NewReader(first.Body)
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			t.Fatal(err)
		}
		if strings.Contains(line, "response.output_text.delta") {
			break
		}
	}
	if _, err = a.stdin.Write([]byte("drain\n")); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(3 * time.Second)
	for {
		probe, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
		if err == nil {
			probe.Close()
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("retired listener retained port")
		}
		time.Sleep(5 * time.Millisecond)
	}
	b := start("b")
	once.Do(func() { close(release) })
	rest, err := io.ReadAll(reader)
	first.Body.Close()
	if err != nil || !strings.Contains(string(rest), "response.completed") {
		t.Fatalf("A lost completion: %s %v", rest, err)
	}
	select {
	case err = <-a.done:
		if err != nil {
			t.Fatal("retired gateway failed", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("retired process did not exit")
	}
	second := send(`{"model":"gpt-5.4","stream":true,"previous_response_id":"response-a","input":[{"role":"user","content":"remember history"},{"type":"reasoning","encrypted_content":"original-cipher"},{"role":"user","content":"continue"}]}`)
	data, err := io.ReadAll(second.Body)
	second.Body.Close()
	if err != nil || !strings.Contains(string(data), "response.completed") {
		t.Fatalf("B continuation failed: %s %v", data, err)
	}
	a.mu.Lock()
	usage, diagnostics := false, false
	for _, e := range a.events {
		if e["type"] == "usage" && e["success"] == true {
			usage = true
		}
		if e["type"] == "request_diagnostics" {
			diagnostics = true
		}
	}
	a.mu.Unlock()
	if !usage || !diagnostics {
		t.Fatalf("late records missing: usage=%v diagnostics=%v", usage, diagnostics)
	}
	mu.Lock()
	defer mu.Unlock()
	if len(upstreamHeaders) != 2 || upstreamHeaders[0].Get("Authorization") != "Bearer upstream-a" || upstreamHeaders[1].Get("Authorization") != "Bearer upstream-b" {
		t.Fatalf("wrong account routing: %d attempts", len(upstreamHeaders))
	}
	if upstreamHeaders[0].Get("Session-Id") == upstreamHeaders[1].Get("Session-Id") || upstreamHeaders[0].Get("Thread-Id") == upstreamHeaders[1].Get("Thread-Id") {
		t.Fatal("account identity reused")
	}
	if !strings.Contains(upstreamBodies[1], "original-cipher") {
		t.Fatal("lost encrypted history")
	}
	b.stdin.Close()
}
