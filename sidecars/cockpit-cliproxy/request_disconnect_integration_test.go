package main

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
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

// This uses a real downstream HTTP socket and an upstream that sends headers
// but no body. Cancellation must work while every stream reader is silent.
func TestClientDisconnectCancelsSilentUpstreamAndReleasesAccount(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, path := range []string{"public-responses", "fixed-provider", "ollama-provider"} {
		t.Run(path, func(t *testing.T) {
			connected, cancelled := make(chan struct{}), make(chan struct{})
			var calls atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if calls.Add(1) == 1 {
					w.Header().Set("Content-Type", "text/event-stream")
					w.WriteHeader(200)
					w.(http.Flusher).Flush()
					close(connected)
					<-r.Context().Done()
					close(cancelled)
					return
				}
				w.Header().Set("Content-Type", "text/event-stream")
				if path == "ollama-provider" {
					_, _ = io.WriteString(w, "data: {\"id\":\"r\",\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
				} else {
					_, _ = io.WriteString(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[]}}\n\n")
				}
			}))
			defer upstream.Close()
			accountID := "disconnect-" + path
			spec := &apiKeySpec{ID: "key", Key: "client-key", Enabled: true, AccountIDs: []string{accountID}}
			account := &accountSpec{ID: accountID, AuthID: accountID}
			m := &manifest{APIKeys: []apiKeySpec{*spec}, ModelIDs: []string{"gpt-5.4"}, MaxAccountConcurrency: 1,
				apiKeyByValue: map[string]*apiKeySpec{spec.Key: spec}, accountByID: map[string]*accountSpec{accountID: account}, accountByAuthID: map[string]*accountSpec{accountID: account}}
			tracker := newRequestUsageTracker()
			policy := &requestPolicy{manifest: m, tracker: tracker}
			server := &relayServer{manifest: m, policy: policy}
			requestPath := "/v1/responses"
			requestBody := `{"model":"gpt-5.4","input":"hi","stream":true}`
			if path == "public-responses" {
				cfg := &config.Config{}
				cfg.Codex.StreamBootstrapBuffering = true
				manager := coreauth.NewManager(nil, &accountSlotSelector{manifest: m, tracker: tracker, fallback: &orderedAuthSelector{order: []string{accountID}}}, &authHook{manifest: m})
				manager.SetConfig(cfg)
				manager.SetRetryConfig(0, 0, 1)
				manager.RegisterExecutor(runtimeexecutor.NewCodexExecutor(cfg))
				registry.GetGlobalRegistry().RegisterClient(accountID, "codex", []*registry.ModelInfo{{ID: "gpt-5.4"}})
				defer registry.GetGlobalRegistry().UnregisterClient(accountID)
				if _, err := manager.Register(context.Background(), &coreauth.Auth{ID: accountID, Provider: "codex", Status: coreauth.StatusActive,
					Attributes: map[string]string{"api_key": "upstream-key", "base_url": upstream.URL}}); err != nil {
					t.Fatal(err)
				}
				server.runtime, server.authManager, server.cfg = manager, manager, cfg
			} else {
				spec.ProviderGateway = &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "upstream-key", WireAPI: "responses", UpstreamModel: "gpt-5.4", UpstreamModels: []string{"gpt-5.4"}}
				if path == "ollama-provider" {
					spec.ProviderGateway.WireAPI = "chat_completions"
					requestPath = "/api/chat"
					requestBody = `{"model":"gpt-5.4","messages":[{"role":"user","content":"hi"}],"stream":true}`
				}
			}
			relay := httptest.NewServer(server.router())
			defer relay.Close()
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			request, _ := http.NewRequestWithContext(ctx, http.MethodPost, relay.URL+requestPath, strings.NewReader(requestBody))
			request.Header.Set("Authorization", "Bearer client-key")
			request.Header.Set("Content-Type", "application/json")
			done := make(chan error, 1)
			go func() {
				response, err := http.DefaultClient.Do(request)
				if response != nil {
					_, _ = io.Copy(io.Discard, response.Body)
					_ = response.Body.Close()
				}
				done <- err
			}()
			select {
			case <-connected:
			case <-time.After(3 * time.Second):
				t.Fatal("upstream never connected")
			}
			if tracker.accountInFlightCount(accountID) != 1 {
				t.Fatal("request did not reserve account concurrency")
			}
			cancel()
			select {
			case <-cancelled:
			case <-time.After(3 * time.Second):
				t.Fatal("downstream disconnect failed to cancel silent upstream")
			}
			select {
			case <-done:
			case <-time.After(time.Second):
				t.Fatal("downstream request did not return")
			}
			deadline := time.Now().Add(time.Second)
			for tracker.accountInFlightCount(accountID) != 0 && time.Now().Before(deadline) {
				time.Sleep(time.Millisecond)
			}
			if tracker.accountInFlightCount(accountID) != 0 {
				t.Fatal("account slot leaked after real disconnect")
			}
			// A second request must be admitted without restarting or resetting state.
			next, _ := http.NewRequest(http.MethodPost, relay.URL+requestPath, strings.NewReader(requestBody))
			next.Header = request.Header.Clone()
			client := &http.Client{Timeout: 3 * time.Second}
			response, err := client.Do(next)
			if err != nil {
				t.Fatal(err)
			}
			_, _ = io.Copy(io.Discard, response.Body)
			_ = response.Body.Close()
			if response.StatusCode != 200 || calls.Load() != 2 {
				t.Fatalf("subsequent request blocked: status=%d calls=%d", response.StatusCode, calls.Load())
			}
		})
	}
}

func TestWebsocketDisconnectCancelsSilentUpstreamAndReleasesAccount(t *testing.T) {
	gin.SetMode(gin.TestMode)
	started, canceled := make(chan struct{}), make(chan struct{})
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(200)
		w.(http.Flusher).Flush()
		close(started)
		<-r.Context().Done()
		close(canceled)
	}))
	defer upstream.Close()
	const accountID = "disconnect-websocket"
	spec := &apiKeySpec{ID: "key", Key: "client-key", Enabled: true, AccountIDs: []string{accountID}, ResponsesWebsockets: true}
	account := &accountSpec{ID: accountID, AuthID: accountID}
	m := &manifest{ModelIDs: []string{"gpt-5.4"}, MaxAccountConcurrency: 1,
		apiKeyByValue: map[string]*apiKeySpec{spec.Key: spec}, accountByID: map[string]*accountSpec{accountID: account}, accountByAuthID: map[string]*accountSpec{accountID: account}}
	tracker := newRequestUsageTracker()
	policy := &requestPolicy{manifest: m, tracker: tracker}
	cfg := &config.Config{}
	cfg.Codex.StreamBootstrapBuffering = true
	// No pings are enabled: closure itself must interrupt synchronous bootstrap.
	cfg.Streaming.KeepAliveSeconds = 0
	manager := coreauth.NewManager(nil, &accountSlotSelector{manifest: m, tracker: tracker, fallback: &orderedAuthSelector{order: []string{accountID}}}, &authHook{manifest: m})
	manager.SetConfig(cfg)
	manager.SetRetryConfig(0, 0, 1)
	manager.RegisterExecutor(runtimeexecutor.NewCodexExecutor(cfg))
	registry.GetGlobalRegistry().RegisterClient(accountID, "codex", []*registry.ModelInfo{{ID: "gpt-5.4"}})
	defer registry.GetGlobalRegistry().UnregisterClient(accountID)
	if _, err := manager.Register(context.Background(), &coreauth.Auth{ID: accountID, Provider: "codex", Status: coreauth.StatusActive,
		Attributes: map[string]string{"api_key": "upstream-key", "base_url": upstream.URL}}); err != nil {
		t.Fatal(err)
	}
	baseHandlers := sdkhandlers.NewBaseAPIHandlers(&cfg.SDKConfig, manager)
	responses := sdkopenai.NewOpenAIResponsesAPIHandler(baseHandlers)
	server := &relayServer{manifest: m, policy: policy, cfg: cfg, runtime: manager, authManager: manager, responsesWebsocket: responses.ResponsesWebsocket}
	relay := httptest.NewServer(server.router())
	defer relay.Close()
	headers := http.Header{"Authorization": {"Bearer client-key"}}
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(relay.URL, "http")+"/v1/responses", headers)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	if err = conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.create","model":"gpt-5.4","input":[{"role":"user","content":[{"type":"input_text","text":"hi"}]}]}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case <-started:
	case <-time.After(3 * time.Second):
		t.Fatal("WebSocket execution did not reach upstream")
	}
	if tracker.accountInFlightCount(accountID) != 1 {
		t.Fatal("WebSocket did not reserve account slot")
	}
	_ = conn.Close()
	select {
	case <-canceled:
	case <-time.After(4 * time.Second):
		t.Fatal("WebSocket disconnect did not cancel silent upstream")
	}
	deadline := time.Now().Add(time.Second)
	for tracker.accountInFlightCount(accountID) != 0 && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if tracker.accountInFlightCount(accountID) != 0 {
		t.Fatal("WebSocket leaked account concurrency")
	}
}
