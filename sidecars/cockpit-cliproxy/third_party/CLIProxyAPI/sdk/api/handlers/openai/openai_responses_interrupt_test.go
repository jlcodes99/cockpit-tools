package openai

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	runtimeexecutor "github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executionregistry"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

// TestResponsesInterruptInFlight verifies that a Codex response.interrupt sent while
// generation is running is forwarded unchanged to the current upstream socket, and that
// the same socket accepts the next response.create. Idle, malformed, and disabled-auth
// interrupts must fail locally without opening or writing that socket.
// Adapted from upstream commit 6331b38bba2cb716480fe78b6638b87db45e521d.
func TestResponsesInterruptInFlight(t *testing.T) {
	for _, testCase := range []struct {
		name       string
		partial    bool
		disconnect bool
		reject     bool
	}{
		{name: "zero_output"},
		{name: "partial_output", partial: true},
		{name: "client_disconnect", disconnect: true},
		{name: "interrupt_not_supported", reject: true},
	} {
		t.Run(testCase.name, func(t *testing.T) {
			interrupt := []byte(`{"type":"response.interrupt","response_id":"r1","mode":"discard_partial_items","extension":"keep"}`)
			const rejected = `{"type":"response.interrupt.failed","response_id":"r1","error":{"code":"interrupt_not_supported","message":"This model does not support response.interrupt."}}`
			var connections atomic.Int32
			upstreamDone := make(chan struct{})
			interruptForwarded := make(chan struct{})
			var upstreamDoneOnce sync.Once
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				connections.Add(1)
				defer upstreamDoneOnce.Do(func() { close(upstreamDone) })
				c, errUpgrade := (&websocket.Upgrader{}).Upgrade(w, r, nil)
				if errUpgrade != nil {
					t.Errorf("upgrade upstream: %v", errUpgrade)
					return
				}
				defer func() { _ = c.Close() }()
				_ = c.SetReadDeadline(time.Now().Add(5 * time.Second))
				read := func() []byte {
					_, payload, errRead := c.ReadMessage()
					if errRead != nil {
						t.Errorf("upstream read: %v", errRead)
					}
					return payload
				}
				write := func(payload string) {
					if errWrite := c.WriteMessage(websocket.TextMessage, []byte(payload)); errWrite != nil {
						t.Errorf("upstream write: %v", errWrite)
					}
				}
				if payload := read(); gjson.GetBytes(payload, "type").String() != "response.create" {
					t.Errorf("expected response.create, got %s", payload)
					return
				}
				write(`{"type":"response.created","response":{"id":"r1"}}`)
				if testCase.partial {
					write(`{"type":"response.output_item.added","output_index":0,"item":{"id":"partial","type":"message","role":"assistant","status":"in_progress","content":[]}}`)
					write(`{"type":"response.output_text.delta","item_id":"partial","output_index":0,"content_index":0,"delta":"unfinished"}`)
				}
				// Hold completion until the in-flight interrupt arrives unchanged.
				for range 2 {
					if payload := read(); !bytes.Equal(payload, interrupt) {
						t.Errorf("interrupt changed: %s", payload)
						return
					}
				}
				close(interruptForwarded)
				if testCase.disconnect {
					_, _, errRead := c.ReadMessage()
					if !websocket.IsUnexpectedCloseError(errRead) && !websocket.IsCloseError(errRead, websocket.CloseNormalClosure, websocket.CloseGoingAway) {
						t.Errorf("upstream socket was not closed after client disconnect: %v", errRead)
					}
					return
				}
				outputTokens := 0
				if testCase.partial {
					outputTokens = 7
				}
				if testCase.reject {
					write(rejected)
					write(`{"type":"response.output_text.delta","item_id":"continued","output_index":0,"content_index":0,"delta":"continued after rejected interrupt"}`)
					write(`{"type":"response.completed","response":{"id":"r1","status":"completed","output":[{"id":"continued","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"continued after rejected interrupt"}]}]}}`)
				} else {
					write(fmt.Sprintf(`{"type":"response.incomplete","response":{"id":"r1","status":"incomplete","incomplete_details":{"reason":"interrupted"},"usage":{"output_tokens":%d},"output":[]}}`, outputTokens))
				}
				if payload := read(); gjson.GetBytes(payload, "type").String() != "response.create" || gjson.GetBytes(payload, "previous_response_id").String() != "r1" || bytes.Contains(payload, []byte("unfinished")) {
					t.Errorf("expected follow-up response.create, got %s", payload)
					return
				}
				write(`{"type":"response.created","response":{"id":"r2"}}`)
				write(`{"type":"response.completed","response":{"id":"r2","status":"completed","output":[]}}`)
				if _, payload, errRead := c.ReadMessage(); !websocket.IsUnexpectedCloseError(errRead) && !websocket.IsCloseError(errRead, websocket.CloseNormalClosure, websocket.CloseGoingAway) {
					t.Errorf("idle Codex socket did not close cleanly or received a late/provider-switch message: %s, error: %v", payload, errRead)
				}
			}))
			defer upstream.Close()

			cfg := &config.Config{}
			manager := coreauth.NewManager(nil, nil, nil)
			manager.SetConfig(cfg)
			manager.RegisterExecutor(runtimeexecutor.NewCodexAutoExecutor(cfg))
			authID := "interrupt-" + testCase.name
			model := "interrupt-model-" + authID
			_, errRegister := manager.Register(context.Background(), &coreauth.Auth{
				ID:       authID,
				Provider: "codex",
				Status:   coreauth.StatusActive,
				Attributes: map[string]string{
					"api_key":    "test-key",
					"base_url":   upstream.URL,
					"websockets": "true",
				},
			})
			if errRegister != nil {
				t.Fatal(errRegister)
			}
			registry.GetGlobalRegistry().RegisterClient(authID, "codex", []*registry.ModelInfo{{ID: model}})
			defer registry.GetGlobalRegistry().UnregisterClient(authID)
			other := &blockingHTTPInterruptExecutor{provider: "xai", canceled: make(chan struct{})}
			manager.RegisterExecutor(other)
			otherAuthID, otherModel := authID+"-xai", model+"-xai"
			if _, errRegister := manager.Register(context.Background(), &coreauth.Auth{ID: otherAuthID, Provider: "xai", Status: coreauth.StatusActive}); errRegister != nil {
				t.Fatal(errRegister)
			}
			registry.GetGlobalRegistry().RegisterClient(otherAuthID, "xai", []*registry.ModelInfo{{ID: otherModel}})
			defer registry.GetGlobalRegistry().UnregisterClient(otherAuthID)

			handler := NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&cfg.SDKConfig, manager))
			router := gin.New()
			router.GET("/v1/responses", handler.ResponsesWebsocket)
			downstream := httptest.NewServer(router)
			defer downstream.Close()

			client, _, errDial := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(downstream.URL, "http")+"/v1/responses", nil)
			if errDial != nil {
				t.Fatal(errDial)
			}
			defer func() { _ = client.Close() }()
			_ = client.SetReadDeadline(time.Now().Add(8 * time.Second))
			send := func(payload []byte) {
				t.Helper()
				if errSend := client.WriteMessage(websocket.TextMessage, payload); errSend != nil {
					t.Fatal(errSend)
				}
			}
			expect := func(event string) []byte {
				t.Helper()
				_, payload, errRead := client.ReadMessage()
				if errRead != nil {
					t.Fatal(errRead)
				}
				if got := gjson.GetBytes(payload, "type").String(); got != event {
					t.Fatalf("expected %s, got %s", event, payload)
				}
				return payload
			}

			send([]byte(`{"type":"response.interrupt"}`))
			expect("error")
			send(interrupt)
			expect("error")
			if connections.Load() != 0 {
				t.Fatal("idle interrupt opened an upstream connection")
			}

			send([]byte(fmt.Sprintf(`{"type":"response.create","model":%q,"input":[]}`, model)))
			expect("response.created")
			if testCase.partial {
				expect("response.output_item.added")
				expect("response.output_text.delta")
			}
			for _, responseID := range []string{`42`, `null`, `""`, `"  "`, `{}`} {
				send([]byte(fmt.Sprintf(`{"type":"response.interrupt","response_id":%s}`, responseID)))
				if payload := expect("error"); gjson.GetBytes(payload, "status").Int() != http.StatusBadRequest {
					t.Fatalf("malformed interrupt status: %s", payload)
				}
			}
			auth, _ := manager.GetByID(authID)
			auth.Disabled = true
			if _, errUpdate := manager.Update(context.Background(), auth); errUpdate != nil {
				t.Fatal(errUpdate)
			}
			send(interrupt)
			expect("error")
			auth.Disabled = false
			if _, errUpdate := manager.Update(context.Background(), auth); errUpdate != nil {
				t.Fatal(errUpdate)
			}
			send(interrupt)
			send(interrupt) // Both interrupts must arrive before the upstream ends this turn.
			if testCase.disconnect {
				select {
				case <-interruptForwarded:
				case <-time.After(time.Second):
					t.Fatal("interrupt did not reach upstream before client disconnect")
				}
				_ = client.Close()
				select {
				case <-upstreamDone:
				case <-time.After(5 * time.Second):
					t.Fatal("upstream did not close after client disconnect")
				}
				if got := connections.Load(); got != 1 {
					t.Fatalf("client disconnect opened %d upstream connections, want 1", got)
				}
				return
			}
			if testCase.reject {
				if payload := expect("response.interrupt.failed"); string(payload) != rejected {
					t.Fatalf("interrupt failure was changed: %s", payload)
				}
				if payload := expect("response.output_text.delta"); gjson.GetBytes(payload, "delta").String() != "continued after rejected interrupt" {
					t.Fatalf("response did not continue after rejected interrupt: %s", payload)
				}
				if payload := expect("response.completed"); gjson.GetBytes(payload, "response.id").String() != "r1" || gjson.GetBytes(payload, "response.output.0.content.0.text").String() != "continued after rejected interrupt" {
					t.Fatalf("original response did not complete: %s", payload)
				}
			} else {
				interrupted := expect("response.incomplete")
				if gjson.GetBytes(interrupted, "response.incomplete_details.reason").String() != "interrupted" || gjson.GetBytes(interrupted, "response.output").Raw != "[]" {
					t.Fatalf("interrupted output was changed: %s", interrupted)
				}
			}
			send([]byte(fmt.Sprintf(`{"type":"response.create","model":%q,"previous_response_id":"r1","input":[]}`, model)))
			expect("response.created")
			expect("response.completed")
			// A normal queued request is a barrier: its validation runs only
			// after the prior forwarding loop has cleared local cancellation.
			send([]byte(`{"type":"unsupported"}`))
			expect("error")
			send(interrupt)
			expect("error") // An interrupt after the second completed turn is idle.
			send([]byte(fmt.Sprintf(`{"type":"response.create","model":%q,"input":[{"role":"user","content":"new provider"}]}`, otherModel)))
			expect("response.created")
			send([]byte(`{"type":"response.interrupt","response_id":"r-http","mode":"discard_partial_items"}`))
			expect("response.incomplete")
			select {
			case <-other.canceled:
			case <-time.After(time.Second):
				t.Fatal("new provider execution was not canceled")
			}
			_ = client.Close()
			select {
			case <-upstreamDone:
			case <-time.After(5 * time.Second):
				t.Fatal("upstream did not close")
			}
			if got := connections.Load(); got != 1 {
				t.Fatalf("upstream connections=%d, want 1", got)
			}
		})
	}
}

// Hold the stream open after its terminal frame to deterministically exercise
// the gap between downstream delivery and forwarding-loop cleanup.
func TestResponsesInterruptRejectsLateFrameAfterTerminalBeforeStreamClose(t *testing.T) {
	stopped := make(chan struct{})
	upstream := make(chan []byte, 1)
	upstream <- []byte(`{"type":"response.completed","response":{"id":"r1","status":"completed","output":[]}}`)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer close(stopped)
		conn, err := responsesWebsocketUpgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		writer := newResponsesWebsocketWriter(conn)
		local := newResponsesLocalInterrupt()
		pump := newResponsesWebsocketReadPump(r.Context(), conn, func(context.Context, []byte) error {
			return coreexecutor.ErrNoActiveUpstreamWebsocket
		}, local, writer)
		defer pump.stop()
		c, _ := gin.CreateTestContext(httptest.NewRecorder())
		c.Request = r.WithContext(pump.ctx)
		_, _, _, _, _ = (*OpenAIResponsesAPIHandler)(nil).forwardResponsesWebsocket(
			c, writer, func(...interface{}) {}, upstream, nil, nil, "late-interrupt",
			responsesWebsocketForwardOptions{localInterrupt: local},
		)
	}))
	defer server.Close()
	client, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = client.Close()
		select {
		case <-stopped:
		case <-time.After(5 * time.Second):
			t.Error("forwarder did not stop after client disconnect")
		}
	}()
	if err := client.SetReadDeadline(time.Now().Add(5 * time.Second)); err != nil {
		t.Fatal(err)
	}
	_, completed, err := client.ReadMessage()
	if err != nil {
		t.Fatal(err)
	}
	if gjson.GetBytes(completed, "type").String() != "response.completed" {
		t.Fatalf("expected terminal frame, got %s", completed)
	}
	if err := client.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.interrupt","response_id":"r1","mode":"discard_partial_items"}`)); err != nil {
		t.Fatal(err)
	}
	_, payload, err := client.ReadMessage()
	if err != nil {
		t.Fatal(err)
	}
	if gjson.GetBytes(payload, "type").String() != "error" {
		t.Fatalf("late interrupt must not synthesize another terminal frame after completion: %s", payload)
	}
}

type homeInterruptDispatcher struct{}

func (homeInterruptDispatcher) HeartbeatOK() bool { return true }

func (homeInterruptDispatcher) RPopAuth(context.Context, string, string, http.Header, int) ([]byte, error) {
	return json.Marshal(coreauth.Auth{
		ID:       "home-interrupt-auth",
		Provider: "codex",
		Status:   coreauth.StatusActive,
		Attributes: map[string]string{
			"websockets": "true",
		},
	})
}

func (homeInterruptDispatcher) AbortAmbiguousDispatch() {}

// homeInterruptExecutor records whether an interrupt was accepted for a Home
// session credential that is not present in the global auth map.
type homeInterruptExecutor struct {
	mu          sync.Mutex
	metadata    []map[string]any
	interrupted [][]byte
	release     chan struct{}
	releaseOnce sync.Once
}

func (*homeInterruptExecutor) Identifier() string { return "codex" }

func (*homeInterruptExecutor) Execute(context.Context, *coreauth.Auth, coreexecutor.Request, coreexecutor.Options) (coreexecutor.Response, error) {
	return coreexecutor.Response{}, errors.New("not implemented")
}

func (e *homeInterruptExecutor) ExecuteStream(ctx context.Context, _ *coreauth.Auth, _ coreexecutor.Request, opts coreexecutor.Options) (*coreexecutor.StreamResult, error) {
	e.mu.Lock()
	e.metadata = append(e.metadata, maps.Clone(opts.Metadata))
	if e.release == nil {
		e.release = make(chan struct{})
	}
	release := e.release
	e.mu.Unlock()
	if lifecycle, ok := opts.ExecutionLifecycle.(interface{ Retain() }); ok {
		lifecycle.Retain()
	}
	chunks := make(chan coreexecutor.StreamChunk, 2)
	go func() {
		defer close(chunks)
		chunks <- coreexecutor.StreamChunk{Payload: []byte(`{"type":"response.created","response":{"id":"r1"}}`)}
		select {
		case <-release:
		case <-ctx.Done():
			return
		}
		chunks <- coreexecutor.StreamChunk{Payload: []byte(`{"type":"response.completed","response":{"id":"r1","status":"completed","output":[]}}`)}
	}()
	return &coreexecutor.StreamResult{Chunks: chunks}, nil
}

func (*homeInterruptExecutor) Refresh(context.Context, *coreauth.Auth) (*coreauth.Auth, error) {
	return nil, errors.New("not implemented")
}

func (*homeInterruptExecutor) CountTokens(context.Context, *coreauth.Auth, coreexecutor.Request, coreexecutor.Options) (coreexecutor.Response, error) {
	return coreexecutor.Response{}, errors.New("not implemented")
}

func (*homeInterruptExecutor) HttpRequest(context.Context, *coreauth.Auth, *http.Request) (*http.Response, error) {
	return nil, errors.New("not implemented")
}

func (e *homeInterruptExecutor) InterruptExecutionSession(ctx context.Context, sessionID string, payload []byte) error {
	if !coreexecutor.WebsocketAuthEnabled(ctx, "home-interrupt-auth") {
		return fmt.Errorf("websocket credential is no longer enabled")
	}
	e.mu.Lock()
	var want string
	if len(e.metadata) > 0 {
		want, _ = e.metadata[0][coreexecutor.ExecutionSessionMetadataKey].(string)
	}
	e.mu.Unlock()
	if want == "" || sessionID != want {
		return fmt.Errorf("interrupt session = %q, want %q", sessionID, want)
	}
	e.mu.Lock()
	e.interrupted = append(e.interrupted, bytes.Clone(payload))
	e.mu.Unlock()
	e.releaseOnce.Do(func() {
		if e.release != nil {
			close(e.release)
		}
	})
	return nil
}

type blockingHTTPInterruptExecutor struct {
	provider string
	calls    atomic.Int32
	canceled chan struct{}
}

func (e *blockingHTTPInterruptExecutor) Identifier() string {
	if e.provider != "" {
		return e.provider
	}
	return "codex"
}

func (*blockingHTTPInterruptExecutor) Execute(context.Context, *coreauth.Auth, coreexecutor.Request, coreexecutor.Options) (coreexecutor.Response, error) {
	return coreexecutor.Response{}, errors.New("not implemented")
}

func (e *blockingHTTPInterruptExecutor) ExecuteStream(ctx context.Context, _ *coreauth.Auth, _ coreexecutor.Request, _ coreexecutor.Options) (*coreexecutor.StreamResult, error) {
	call := e.calls.Add(1)
	chunks := make(chan coreexecutor.StreamChunk, 2)
	go func() {
		defer close(chunks)
		if call > 1 {
			chunks <- coreexecutor.StreamChunk{Payload: []byte(`{"type":"response.created","response":{"id":"r-http-2"}}`)}
			chunks <- coreexecutor.StreamChunk{Payload: []byte(`{"type":"response.completed","response":{"id":"r-http-2","status":"completed","output":[]}}`)}
			return
		}
		chunks <- coreexecutor.StreamChunk{Payload: []byte(`{"type":"response.created","response":{"id":"r-http"}}`)}
		<-ctx.Done()
		close(e.canceled)
	}()
	return &coreexecutor.StreamResult{Chunks: chunks}, nil
}

func (*blockingHTTPInterruptExecutor) Refresh(context.Context, *coreauth.Auth) (*coreauth.Auth, error) {
	return nil, errors.New("not implemented")
}

func (*blockingHTTPInterruptExecutor) CountTokens(context.Context, *coreauth.Auth, coreexecutor.Request, coreexecutor.Options) (coreexecutor.Response, error) {
	return coreexecutor.Response{}, errors.New("not implemented")
}

func (*blockingHTTPInterruptExecutor) HttpRequest(context.Context, *coreauth.Auth, *http.Request) (*http.Response, error) {
	return nil, errors.New("not implemented")
}

func (*blockingHTTPInterruptExecutor) InterruptExecutionSession(context.Context, string, []byte) error {
	return coreexecutor.ErrNoActiveUpstreamWebsocket
}

// TestResponsesInterruptStopsHTTPUpstream verifies that an in-flight interrupt
// cancels an HTTP upstream turn and lets the same socket continue.
func TestResponsesInterruptStopsHTTPUpstream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	executor := &blockingHTTPInterruptExecutor{canceled: make(chan struct{})}
	manager := coreauth.NewManager(nil, nil, nil)
	manager.SetConfig(&config.Config{})
	manager.RegisterExecutor(executor)
	const authID = "interrupt-http-auth"
	const model = "interrupt-http-model"
	_, errRegister := manager.Register(context.Background(), &coreauth.Auth{
		ID:         authID,
		Provider:   "codex",
		Status:     coreauth.StatusActive,
		Attributes: map[string]string{"api_key": "test-key"},
	})
	if errRegister != nil {
		t.Fatal(errRegister)
	}
	registry.GetGlobalRegistry().RegisterClient(authID, "codex", []*registry.ModelInfo{{ID: model}})
	defer registry.GetGlobalRegistry().UnregisterClient(authID)

	handler := NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&config.SDKConfig{}, manager))
	router := gin.New()
	router.GET("/v1/responses", handler.ResponsesWebsocket)
	downstream := httptest.NewServer(router)
	defer downstream.Close()
	client, _, errDial := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(downstream.URL, "http")+"/v1/responses", nil)
	if errDial != nil {
		t.Fatal(errDial)
	}
	defer func() { _ = client.Close() }()
	_ = client.SetReadDeadline(time.Now().Add(5 * time.Second))
	if errSend := client.WriteMessage(websocket.TextMessage, []byte(fmt.Sprintf(`{"type":"response.create","model":%q,"input":[]}`, model))); errSend != nil {
		t.Fatal(errSend)
	}
	_, created, errRead := client.ReadMessage()
	if errRead != nil {
		t.Fatal(errRead)
	}
	if got := gjson.GetBytes(created, "type").String(); got != "response.created" {
		t.Fatalf("expected response.created, got %s", created)
	}
	if errSend := client.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.interrupt","response_id":"r-http","mode":"discard_partial_items"}`)); errSend != nil {
		t.Fatal(errSend)
	}
	_, interrupted, errInterrupted := client.ReadMessage()
	if errInterrupted != nil {
		t.Fatal(errInterrupted)
	}
	if got := gjson.GetBytes(interrupted, "type").String(); got != "response.incomplete" {
		t.Fatalf("expected response.incomplete, got %s", interrupted)
	}
	if got := gjson.GetBytes(interrupted, "response.id").String(); got != "r-http" {
		t.Fatalf("interrupted response id = %q", got)
	}
	if got := gjson.GetBytes(interrupted, "response.incomplete_details.reason").String(); got != "interrupted" {
		t.Fatalf("interrupt reason = %q, payload %s", got, interrupted)
	}
	select {
	case <-executor.canceled:
	case <-time.After(time.Second):
		t.Fatal("http upstream was not canceled")
	}
	if errSend := client.WriteMessage(websocket.TextMessage, []byte(fmt.Sprintf(`{"type":"response.create","model":%q,"previous_response_id":"r-http","input":[]}`, model))); errSend != nil {
		t.Fatal(errSend)
	}
	_, followCreated, errFollow := client.ReadMessage()
	if errFollow != nil {
		t.Fatal(errFollow)
	}
	if got := gjson.GetBytes(followCreated, "type").String(); got != "response.created" {
		t.Fatalf("expected follow-up response.created, got %s", followCreated)
	}
	_, followDone, errDone := client.ReadMessage()
	if errDone != nil {
		t.Fatal(errDone)
	}
	if got := gjson.GetBytes(followDone, "type").String(); got != "response.completed" {
		t.Fatalf("expected follow-up response.completed, got %s", followDone)
	}
}

// TestResponsesInterruptUsesHomeSessionAuth verifies that a credential staged only
// on the Home execution session can still authorize response.interrupt.
func TestResponsesInterruptUsesHomeSessionAuth(t *testing.T) {
	gin.SetMode(gin.TestMode)
	manager := coreauth.NewManager(nil, nil, nil)
	manager.SetConfig(&config.Config{Home: config.HomeConfig{Enabled: true}})
	manager.PublishHomeDispatch(homeInterruptDispatcher{}, executionregistry.New(), 1)
	executor := &homeInterruptExecutor{}
	manager.RegisterExecutor(executor)
	const authID = "home-interrupt-auth"
	const model = "home-interrupt-model"
	registry.GetGlobalRegistry().RegisterClient(authID, "codex", []*registry.ModelInfo{{ID: model}})
	defer registry.GetGlobalRegistry().UnregisterClient(authID)

	handler := NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&config.SDKConfig{}, manager))
	router := gin.New()
	router.GET("/v1/responses", handler.ResponsesWebsocket)
	downstream := httptest.NewServer(router)
	defer downstream.Close()

	client, _, errDial := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(downstream.URL, "http")+"/v1/responses", nil)
	if errDial != nil {
		t.Fatal(errDial)
	}
	defer func() { _ = client.Close() }()
	_ = client.SetReadDeadline(time.Now().Add(8 * time.Second))
	if errSend := client.WriteMessage(websocket.TextMessage, []byte(fmt.Sprintf(`{"type":"response.create","model":%q,"input":[]}`, model))); errSend != nil {
		t.Fatal(errSend)
	}
	_, payload, errRead := client.ReadMessage()
	if errRead != nil {
		t.Fatal(errRead)
	}
	if got := gjson.GetBytes(payload, "type").String(); got != "response.created" {
		t.Fatalf("expected response.created, got %s", payload)
	}
	if _, ok := manager.GetByID(authID); ok {
		t.Fatal("home credential leaked into the global auth map")
	}
	executor.mu.Lock()
	if len(executor.metadata) != 1 {
		executor.mu.Unlock()
		t.Fatalf("executor calls = %d, want 1", len(executor.metadata))
	}
	sessionID, _ := executor.metadata[0][coreexecutor.ExecutionSessionMetadataKey].(string)
	executor.mu.Unlock()
	if _, ok := manager.GetExecutionSessionAuthByID(sessionID, authID); !ok {
		t.Fatal("home session auth was not staged")
	}

	interrupt := []byte(`{"type":"response.interrupt","response_id":"r1","mode":"discard_partial_items","extension":"keep"}`)
	if errSend := client.WriteMessage(websocket.TextMessage, interrupt); errSend != nil {
		t.Fatal(errSend)
	}
	_, completed, errCompleted := client.ReadMessage()
	if errCompleted != nil {
		t.Fatal(errCompleted)
	}
	if got := gjson.GetBytes(completed, "type").String(); got != "response.completed" {
		t.Fatalf("expected response.completed after interrupt, got %s", completed)
	}
	executor.mu.Lock()
	gotInterrupt := append([][]byte(nil), executor.interrupted...)
	executor.mu.Unlock()
	if len(gotInterrupt) != 1 || !bytes.Equal(gotInterrupt[0], interrupt) {
		t.Fatalf("interrupt payload = %#v, want %s", gotInterrupt, interrupt)
	}
	if errSend := client.WriteMessage(websocket.TextMessage, []byte(fmt.Sprintf(`{"type":"response.create","model":%q,"input":[]}`, model))); errSend != nil {
		t.Fatal(errSend)
	}
	_, followCreated, errFollow := client.ReadMessage()
	if errFollow != nil {
		t.Fatal(errFollow)
	}
	if got := gjson.GetBytes(followCreated, "type").String(); got != "response.created" {
		t.Fatalf("expected follow-up response.created, got %s", followCreated)
	}
	_, followDone, errDone := client.ReadMessage()
	if errDone != nil {
		t.Fatal(errDone)
	}
	if got := gjson.GetBytes(followDone, "type").String(); got != "response.completed" {
		t.Fatalf("expected follow-up response.completed, got %s", followDone)
	}
}
