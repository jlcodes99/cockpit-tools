package executor

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestCodexBootstrapStructuredAccountFailoverAndCommittedStreamBoundary(t *testing.T) {
	for _, transport := range []string{"http", "websocket"} {
		for _, tc := range []struct {
			name, event string
			enabled     bool
			outputFirst bool
			wantRotate  bool
		}{
			{"usage", `{"type":"response.failed","response":{"error":{"type":"usage_limit_reached","message":"quota used","resets_in_seconds":30}}}`, true, false, true},
			{"insufficient", `{"type":"error","error":{"type":"invalid_request_error","code":"insufficient_quota","message":"quota used"}}`, true, false, true},
			{"deactivated", `{"type":"response.failed","response":{"error":{"type":"invalid_request_error","code":"account_deactivated","message":"account disabled"}}}`, true, false, true},
			{"disabled", `{"type":"error","error":{"code":"insufficient_quota","message":"quota used"}}`, false, false, false},
			{"committed", `{"type":"error","error":{"code":"insufficient_quota","message":"quota used"}}`, true, true, false},
			{"invalid-input", `{"type":"error","error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"input too long"}}`, true, false, false},
			{"ordinary-text", `{"type":"response.output_text.delta","delta":"usage_limit_reached insufficient_quota account_deactivated"}`, true, false, false},
		} {
			t.Run(transport+"/"+tc.name, func(t *testing.T) {
				var callsA, callsB atomic.Int32
				upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
				upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					first := r.Header.Get("Authorization") == "Bearer a-key"
					if first {
						callsA.Add(1)
					} else {
						callsB.Add(1)
					}
					events := []string{codexCreatedEvent, codexInProgressEvent}
					if first {
						if tc.outputFirst {
							events = append(events, codexOutputAddedEvent)
						}
						events = append(events, tc.event)
						if tc.name == "ordinary-text" {
							events = append(events, codexCompletedEventBody)
						}
					} else {
						events = append(events, codexOutputAddedEvent, codexCompletedEventBody)
					}
					if transport == "websocket" {
						conn, err := upgrader.Upgrade(w, r, nil)
						if err != nil {
							t.Errorf("websocket upgrade: %v", err)
							return
						}
						defer func() { _ = conn.Close() }()
						if _, _, errRead := conn.ReadMessage(); errRead != nil {
							t.Errorf("websocket request: %v", errRead)
							return
						}
						for _, event := range events {
							_ = conn.WriteMessage(websocket.TextMessage, []byte(event))
						}
						return
					}
					w.Header().Set("Content-Type", "text/event-stream")
					for _, event := range events {
						_, _ = fmt.Fprintf(w, "data: %s\n\n", event)
					}
				}))
				defer upstream.Close()
				cfg := codexBufferingConfig(tc.enabled)
				manager := coreauth.NewManager(nil, nil, nil)
				manager.SetConfig(cfg)
				manager.SetRetryConfig(0, 0, 2)
				if transport == "websocket" {
					manager.RegisterExecutor(NewCodexWebsocketsExecutor(cfg))
				} else {
					manager.RegisterExecutor(NewCodexExecutor(cfg))
				}
				for i, name := range []string{"a", "b"} {
					id := "bootstrap-" + transport + "-" + tc.name + "-" + name
					registry.GetGlobalRegistry().RegisterClient(id, "codex", []*registry.ModelInfo{{ID: "gpt-5.6-terra"}})
					defer registry.GetGlobalRegistry().UnregisterClient(id)
					_, err := manager.Register(context.Background(), &coreauth.Auth{ID: id, Provider: "codex", Status: coreauth.StatusActive,
						Attributes: map[string]string{"api_key": name + "-key", "base_url": upstream.URL, "priority": fmt.Sprint(100 - i)}})
					if err != nil {
						t.Fatal(err)
					}
				}
				req, opts := codexTestRequest()
				if transport == "websocket" {
					req, opts = codexWebsocketRequest()
				}
				result, err := manager.ExecuteStream(context.Background(), []string{"codex"}, req, opts)
				var body string
				var streamErr error
				if result != nil {
					body, streamErr = drainChunks(result)
				}
				if tc.wantRotate {
					if err != nil || streamErr != nil || !strings.Contains(body, "response.completed") || callsA.Load() != 1 || callsB.Load() != 1 {
						t.Fatalf("failover failed: err=%v streamErr=%v a=%d b=%d body=%s", err, streamErr, callsA.Load(), callsB.Load(), body)
					}
				} else if callsA.Load() != 1 || callsB.Load() != 0 {
					t.Fatalf("committed/input/disabled stream replayed: a=%d b=%d err=%v streamErr=%v", callsA.Load(), callsB.Load(), err, streamErr)
				}
			})
		}
	}
}

func TestCodexBootstrapByteBoundReleasesLargeHandshake(t *testing.T) {
	event := `{"type":"response.in_progress","response":{"id":"large","padding":"` + strings.Repeat("x", codexBootstrapMaxBufferedBytes) + `"}}`
	server := codexSSEServer(codexCreatedEvent, event, `{"type":"error","error":{"code":"insufficient_quota","message":"quota used"}}`)
	defer server.Close()
	req, opts := codexTestRequest()
	result, err := NewCodexExecutor(codexBufferingConfig(true)).ExecuteStream(context.Background(), codexTestAuth(server.URL), req, opts)
	if err != nil || result == nil {
		t.Fatalf("large bootstrap stayed buffered: %v", err)
	}
	body, _ := drainChunks(result)
	if !strings.Contains(body, "response.created") {
		t.Fatal("byte-bound did not release buffered events")
	}
}

var _ cliproxyexecutor.StatusError = statusErr{}
