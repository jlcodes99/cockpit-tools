package executor

import (
	"context"
	"fmt"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	auth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	api "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestCodexQueuedFailureSurvivesReaderDisconnect(t *testing.T) {
	for _, backlog := range []bool{false, true} {
		for _, status := range []int{401, 429} {
			for _, buffering := range []bool{false, true} {
				for _, stream := range []bool{false, true} {
					name := "execute"
					if stream {
						name = "stream"
					}
					t.Run(fmt.Sprintf("%s/status_%d/buffering_%v/backlog_%v", name, status, buffering, backlog), func(t *testing.T) {
						ctx, stop := context.WithTimeout(t.Context(), 5*time.Second)
						defer stop()
						ctx, cancel := context.WithCancel(ctx)
						defer cancel()
						gate := make(chan struct{})
						up := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
						srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
							conn, err := up.Upgrade(w, r, nil)
							if err != nil {
								t.Error(err)
								return
							}
							defer conn.Close()
							if _, _, err := conn.ReadMessage(); err != nil {
								t.Error(err)
								return
							}
							if err := conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.created","response":{"id":"resp_review"}}`)); err != nil {
								t.Error(err)
								return
							}
							if backlog {
								if err := conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.output_text.delta","delta":"first","item_id":"msg_1","output_index":0,"content_index":0}`)); err != nil {
									t.Error(err)
									return
								}
							}
							select {
							case <-gate:
							case <-ctx.Done():
								return
							}
							failure := fmt.Sprintf(`{"type":"error","status":%d,"body":{"error":{"type":"usage_limit_reached","message":"usage limit reached","resets_in_seconds":600}}}`, status)
							if status == 401 {
								failure = `{"type":"error","status":401,"body":{"error":{"type":"authentication_error","message":"credential rejected"}}}`
							}
							if err := conn.WriteMessage(websocket.TextMessage, []byte(failure)); err != nil {
								t.Error(err)
							}
						}))
						defer srv.Close()
						cfg := &config.Config{Codex: config.CodexConfig{StreamBootstrapBuffering: buffering}}
						cfg.DisableImageGeneration = config.DisableImageGenerationAll
						exec := NewCodexWebsocketsExecutor(cfg)
						exec.store = &codexWebsocketSessionStore{sessions: make(map[string]*codexWebsocketSession)}
						const id = "queued-quota-review"
						disconnect := exec.UpstreamDisconnectChan(id)
						go func() {
							select {
							case <-disconnect:
								cancel()
							case <-ctx.Done():
							}
						}()
						req, opts := codexWebsocketRequest()
						opts.Metadata = map[string]any{api.ExecutionSessionMetadataKey: id}
						opts.WebSocketResponseObserver = func(_ context.Context, event api.WebSocketResponseEvent) {
							target := "response.created"
							if backlog {
								target = "response.output_text.delta"
							}
							if event.EventType != target {
								return
							}
							close(gate)
							<-ctx.Done()
						}
						var executionErr error
						credentials := &auth.Auth{ID: "review", Provider: "codex", Attributes: map[string]string{"base_url": srv.URL, "api_key": "test"}}
						if stream {
							result, err := exec.ExecuteStream(ctx, credentials, req, opts)
							executionErr = err
							if result != nil {
								<-ctx.Done()
								// No downstream reader is running yet. The producer must release the
								// session without waiting for a consumer of canceled normal output.
								sess := exec.getOrCreateSession(id)
								deadline := time.Now().Add(time.Second)
								released := false
								for time.Now().Before(deadline) {
									if sess.reqMu.TryLock() {
										sess.reqMu.Unlock()
										released = true
										break
									}
									time.Sleep(time.Millisecond)
								}
								if !released {
									t.Fatal("failure producer blocked waiting for a client consumer")
								}
								for chunk := range result.Chunks {
									if chunk.Err != nil {
										executionErr = chunk.Err
									}
								}
							}
						} else {
							_, executionErr = exec.Execute(ctx, credentials, req, opts)
						}
						typed, ok := executionErr.(interface{ StatusCode() int })
						if !ok || typed.StatusCode() != status {
							t.Fatalf("queued %d lost: %v", status, executionErr)
						}
						if status == 429 {
							retry, ok := executionErr.(interface{ RetryAfter() *time.Duration })
							if !ok || retry.RetryAfter() == nil || *retry.RetryAfter() != 600*time.Second {
								t.Fatalf("queued quota retry lost: %v", executionErr)
							}
						}
					})
				}
			}
		}
	}

}
