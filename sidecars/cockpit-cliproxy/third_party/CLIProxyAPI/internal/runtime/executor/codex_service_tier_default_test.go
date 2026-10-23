package executor

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

func TestCodexServiceTierDefaultFinalUpstreamBody(t *testing.T) {
	for _, apiKey := range []bool{false, true} {
		for _, stream := range []bool{false, true} {
			t.Run(fmt.Sprintf("apiKey=%v/stream=%v", apiKey, stream), func(t *testing.T) {
				bodies := make(chan []byte, 4)
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					body, _ := io.ReadAll(r.Body)
					bodies <- body
					w.Header().Set("Content-Type", "text/event-stream")
					_, _ = fmt.Fprintf(w, "data: %s\n\ndata: %s\n\n", codexCreatedEvent, codexCompletedEventBody)
				}))
				defer server.Close()
				cfg := &config.Config{Payload: config.PayloadConfig{Default: []config.PayloadRule{{Models: []config.PayloadModelRule{{Name: "*", Protocol: "codex"}}, Params: map[string]any{"service_tier": "priority"}}}}}
				exec := NewCodexExecutor(cfg)
				auth := apiServiceTestAuth(server.URL)
				if apiKey {
					auth.Attributes["api_key"] = "test"
				}
				var current atomic.Value
				current.Store("")
				parent := coreexecutor.WithServiceTierDefaultProvider(context.Background(), func() string { return current.Load().(string) })
				for _, tier := range []string{"", "priority", ""} {
					current.Store(tier)
					ctx := coreexecutor.SnapshotServiceTierDefault(parent)
					req, opts := codexTestRequest()
					if stream {
						result, err := exec.ExecuteStream(ctx, auth, req, opts)
						if err != nil {
							t.Fatal(err)
						}
						if _, err := drainChunks(result); err != nil {
							t.Fatal(err)
						}
					} else {
						if _, err := exec.Execute(ctx, auth, req, opts); err != nil {
							t.Fatal(err)
						}
					}
					body := <-bodies
					if got := gjson.GetBytes(body, "service_tier").String(); got != tier {
						t.Fatalf("upstream tier=%q want=%q body=%s", got, tier, body)
					}
				}
			})
		}
	}
}

func TestCodexServiceTierDefaultKeepsInFlightStream(t *testing.T) {
	bodies := make(chan []byte, 2)
	started := make(chan struct{})
	finish := make(chan struct{})
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		bodies <- body
		w.Header().Set("Content-Type", "text/event-stream")
		if calls.Add(1) == 1 {
			_, _ = fmt.Fprintf(w, "data: %s\n\n", codexCreatedEvent)
			w.(http.Flusher).Flush()
			close(started)
			<-finish
		}
		_, _ = fmt.Fprintf(w, "data: %s\n\n", codexCompletedEventBody)
	}))
	defer server.Close()
	var current atomic.Value
	current.Store("priority")
	parent := coreexecutor.WithServiceTierDefaultProvider(context.Background(), func() string { return current.Load().(string) })
	ctx := coreexecutor.SnapshotServiceTierDefault(parent)
	exec := NewCodexExecutor(&config.Config{})
	req, opts := codexTestRequest()
	result, err := exec.ExecuteStream(ctx, apiServiceTestAuth(server.URL), req, opts)
	if err != nil {
		close(finish)
		t.Fatal(err)
	}
	<-started
	current.Store("")
	next, err := exec.ExecuteStream(coreexecutor.SnapshotServiceTierDefault(parent), apiServiceTestAuth(server.URL), req, opts)
	close(finish)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := drainChunks(next); err != nil {
		t.Fatal(err)
	}
	if _, err := drainChunks(result); err != nil {
		t.Fatalf("in-flight stream interrupted: %v", err)
	}
	if first := gjson.GetBytes(<-bodies, "service_tier").String(); first != "priority" {
		t.Fatalf("in-flight tier = %q", first)
	}
	if second := gjson.GetBytes(<-bodies, "service_tier").String(); second != "" {
		t.Fatalf("next tier = %q", second)
	}
}

func TestCodexServiceTierDefaultWebsocketUpstreamBody(t *testing.T) {
	for _, stream := range []bool{false, true} {
		t.Run(fmt.Sprintf("stream=%v", stream), func(t *testing.T) {
			bodies := make(chan []byte, 3)
			var connections atomic.Int32
			upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				conn, err := upgrader.Upgrade(w, r, nil)
				if err != nil {
					t.Errorf("upgrade: %v", err)
					return
				}
				connections.Add(1)
				defer conn.Close()
				for {
					_, body, err := conn.ReadMessage()
					if err != nil {
						return
					}
					bodies <- body
					if err := conn.WriteMessage(websocket.TextMessage, []byte(codexCreatedEvent)); err != nil {
						return
					}
					if err := conn.WriteMessage(websocket.TextMessage, []byte(codexCompletedEventBody)); err != nil {
						return
					}
				}
			}))
			defer server.Close()
			cfg := &config.Config{Payload: config.PayloadConfig{Default: []config.PayloadRule{{Models: []config.PayloadModelRule{{Name: "*", Protocol: "codex"}}, Params: map[string]any{"service_tier": "priority"}}}}}
			exec := NewCodexWebsocketsExecutor(cfg)
			defer exec.CloseExecutionSession("tier-upstream-ws")
			var current atomic.Value
			current.Store("")
			parent := coreexecutor.WithServiceTierDefaultProvider(context.Background(), func() string { return current.Load().(string) })
			for _, tier := range []string{"", "priority", ""} {
				current.Store(tier)
				ctx := coreexecutor.SnapshotServiceTierDefault(parent)
				req, opts := codexTestRequest()
				opts.Metadata = map[string]any{coreexecutor.ExecutionSessionMetadataKey: "tier-upstream-ws"}
				if stream {
					result, err := exec.ExecuteStream(ctx, apiServiceTestAuth(server.URL), req, opts)
					if err != nil {
						t.Fatal(err)
					}
					if _, err := drainChunks(result); err != nil {
						t.Fatal(err)
					}
				} else {
					if _, err := exec.Execute(ctx, apiServiceTestAuth(server.URL), req, opts); err != nil {
						t.Fatal(err)
					}
				}
				body := <-bodies
				if got := gjson.GetBytes(body, "service_tier").String(); got != tier {
					t.Fatalf("upstream WS tier=%q,want=%q body=%s", got, tier, body)
				}
			}
			if got := connections.Load(); got != 1 {
				t.Fatalf("upstream connections=%d,want 1", got)
			}
		})
	}
}
