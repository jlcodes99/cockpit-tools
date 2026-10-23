package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	runtimeexecutor "github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
	sdkhandlers "github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	sdkopenai "github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers/openai"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

type pausedRuntimeFixture struct {
	m           *manifest
	spec        *apiKeySpec
	router      http.Handler
	mu          sync.Mutex
	sent        []string
	failed      map[string]bool
	connections int
}

func newPausedRuntimeFixture(t *testing.T, paused, scope []string) *pausedRuntimeFixture {
	t.Helper()
	gin.SetMode(gin.TestMode)
	f := &pausedRuntimeFixture{failed: map[string]bool{}}
	upstream := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		account := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer fixture-")
		f.mu.Lock()
		f.sent = append(f.sent, account)
		failed := f.failed[account]
		f.mu.Unlock()
		if failed {
			w.WriteHeader(http.StatusServiceUnavailable)
			_, _ = io.WriteString(w, `{"error":{"message":"fixture unavailable"}}`)
			return
		}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = fmt.Fprintf(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"pause-%s\",\"model\":\"gpt-5.4\",\"object\":\"response\",\"status\":\"completed\",\"output\":[]}}\n\n", account)
	}))
	upstream.Config.ConnState = func(_ net.Conn, state http.ConnState) {
		if state == http.StateNew {
			f.mu.Lock()
			f.connections++
			f.mu.Unlock()
		}
	}
	upstream.Start()
	t.Cleanup(upstream.Close)
	accounts := []accountSpec{{ID: "a", AuthID: "pause-a.json"}, {ID: "b", AuthID: "pause-b.json"}, {ID: "c", AuthID: "pause-c.json"}}
	payload, err := json.Marshal(manifest{Accounts: accounts, PausedAccountIDs: paused, ModelIDs: []string{"gpt-5.4"}, APIKeys: []apiKeySpec{{ID: "client", Key: "client-key", Enabled: true, ResponsesWebsockets: true, AccountIDs: scope}}})
	if err != nil {
		t.Fatal(err)
	}
	f.m = loadAutomaticRoutingManifest(t, string(payload))
	f.spec = f.m.apiKeyByValue["client-key"]
	cfg := &config.Config{}
	selector := buildCoreAuthSelector(nil, &orderedAuthSelector{order: []string{"pause-a.json", "pause-b.json", "pause-c.json"}}, f.m, nil)
	manager := coreauth.NewManager(nil, selector, nil)
	manager.SetConfig(cfg)
	manager.SetRetryConfig(0, 0, 3)
	manager.RegisterExecutor(runtimeexecutor.NewCodexExecutor(cfg))
	for _, account := range accounts {
		auth := &coreauth.Auth{ID: account.AuthID, Provider: "codex", Status: coreauth.StatusActive, Attributes: map[string]string{"account_id": account.ID, "api_key": "fixture-" + account.ID, "base_url": upstream.URL}}
		if _, err := manager.Register(context.Background(), auth); err != nil {
			t.Fatal(err)
		}
		registry.GetGlobalRegistry().RegisterClient(auth.ID, "codex", []*registry.ModelInfo{{ID: "gpt-5.4"}})
		t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(auth.ID) })
	}
	f.m.authManager = manager
	responses := sdkopenai.NewOpenAIResponsesAPIHandler(sdkhandlers.NewBaseAPIHandlers(&cfg.SDKConfig, manager))
	f.router = (&relayServer{manifest: f.m, cfg: cfg, runtime: manager, authManager: manager, policy: &requestPolicy{manifest: f.m}, responsesWebsocket: responses.ResponsesWebsocket}).router()
	return f
}

func (f *pausedRuntimeFixture) requests() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]string(nil), f.sent...)
}

func TestPausedAccountsRealRuntimeHTTPAndSSE(t *testing.T) {
	for _, stream := range []bool{false, true} {
		for _, allPaused := range []bool{false, true} {
			t.Run(fmt.Sprintf("stream=%t/all=%t", stream, allPaused), func(t *testing.T) {
				paused := []string{"a"}
				if allPaused {
					paused = append(paused, "b")
				}
				f := newPausedRuntimeFixture(t, paused, []string{"a", "b"})
				r := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(fmt.Sprintf(`{"model":"gpt-5.4","input":"fixture","stream":%t}`, stream)))
				r.Header.Set("Authorization", "Bearer client-key")
				r.Header.Set("Content-Type", "application/json")
				w := httptest.NewRecorder()
				f.router.ServeHTTP(w, r)
				if allPaused {
					var rejection struct {
						Error struct {
							Code string `json:"code"`
						} `json:"error"`
					}
					if err := json.Unmarshal(w.Body.Bytes(), &rejection); err != nil {
						t.Fatal(err)
					}
					if w.Code != 503 || rejection.Error.Code != "account_paused" || len(f.requests()) != 0 {
						t.Fatalf("paused scope escaped: status=%d sent=%v body=%s", w.Code, f.requests(), w.Body.String())
					}
				} else if w.Code != 200 || strings.Join(f.requests(), ",") != "b" || !strings.Contains(w.Body.String(), "pause-b") {
					t.Fatalf("active fallback failed: status=%d sent=%v body=%s", w.Code, f.requests(), w.Body.String())
				}
				if strings.Join(f.spec.AccountIDs, ",") != "a,b" || len(f.m.Accounts) != 3 {
					t.Fatal("scope or members changed")
				}
			})
		}
	}
}

func TestPausedAccountsRealResponsesWebSocket(t *testing.T) {
	for _, allPaused := range []bool{false, true} {
		t.Run(fmt.Sprint(allPaused), func(t *testing.T) {
			paused := []string{"a"}
			if allPaused {
				paused = append(paused, "b")
			}
			f := newPausedRuntimeFixture(t, paused, []string{"a", "b"})
			server := httptest.NewServer(f.router)
			defer server.Close()
			header := http.Header{"Authorization": []string{"Bearer client-key"}}
			ws, response, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http")+"/v1/responses", header)
			if err != nil {
				t.Fatalf("upgrade: %v (%v)", err, response)
			}
			defer ws.Close()
			_ = ws.SetReadDeadline(time.Now().Add(5 * time.Second))
			if err := ws.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.create","model":"gpt-5.4","input":[{"role":"user","content":"fixture"}]}`)); err != nil {
				t.Fatal(err)
			}
			for {
				_, payload, err := ws.ReadMessage()
				if err != nil {
					t.Fatal(err)
				}
				if allPaused {
					var event struct {
						Type   string `json:"type"`
						Status int    `json:"status"`
						Error  struct {
							Code      string `json:"code"`
							Type      string `json:"type"`
							Retryable *bool  `json:"retryable"`
						} `json:"error"`
					}
					if err := json.Unmarshal(payload, &event); err != nil {
						t.Fatal(err)
					}
					f.mu.Lock()
					connections := f.connections
					f.mu.Unlock()
					if event.Type != "error" || event.Status != 503 || event.Error.Code != "account_paused" || event.Error.Type != "invalid_request_error" || event.Error.Retryable == nil || *event.Error.Retryable || len(f.requests()) != 0 || connections != 0 {
						t.Fatalf("paused WS scope escaped: sent=%v payload=%s", f.requests(), payload)
					}
					break
				}
				if strings.Contains(string(payload), "response.completed") {
					if strings.Join(f.requests(), ",") != "b" || !strings.Contains(string(payload), "pause-b") {
						t.Fatalf("wrong WS account: sent=%v payload=%s", f.requests(), payload)
					}
					break
				}
			}
		})
	}
}

func TestPausedAccountNotRetriedAfterActiveAccountFails(t *testing.T) {
	f := newPausedRuntimeFixture(t, []string{"b"}, []string{"a", "b"})
	f.mu.Lock()
	f.failed["a"] = true
	f.mu.Unlock()
	r := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.4","input":"fixture"}`))
	r.Header.Set("Authorization", "Bearer client-key")
	r.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	f.router.ServeHTTP(w, r)
	if w.Code < 400 || len(f.requests()) == 0 {
		t.Fatalf("expected active failure: %d %s", w.Code, w.Body.String())
	}
	for _, account := range f.requests() {
		if account != "a" {
			t.Fatalf("retry escaped to paused/out-of-scope account: %v", f.requests())
		}
	}
}
