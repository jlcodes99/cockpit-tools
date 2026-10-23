package openai

import (
	"context"
	"errors"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdkconfig "github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

type retiringWebsocketExecutor struct {
	homeResponsesWebsocketExecutor
	entered chan context.Context
	release chan struct{}
	calls   atomic.Int32
}

func (e *retiringWebsocketExecutor) ExecuteStream(ctx context.Context, _ *coreauth.Auth, _ coreexecutor.Request, _ coreexecutor.Options) (*coreexecutor.StreamResult, error) {
	e.calls.Add(1)
	e.entered <- ctx
	chunks := make(chan coreexecutor.StreamChunk, 1)
	go func() {
		defer close(chunks)
		select {
		case <-e.release:
			chunks <- coreexecutor.StreamChunk{Payload: []byte(`{"type":"response.completed","response":{"id":"drained-response","output":[]}}`)}
		case <-ctx.Done():
		}
	}()
	return &coreexecutor.StreamResult{Chunks: chunks}, nil
}
func TestGatewayDrainFinishesCurrentWebsocketTurnAndRejectsQueuedTurn(t *testing.T) {
	gin.SetMode(gin.TestMode)
	executor := &retiringWebsocketExecutor{entered: make(chan context.Context, 1), release: make(chan struct{})}
	manager := coreauth.NewManager(nil, nil, nil)
	manager.RegisterExecutor(executor)
	auth := &coreauth.Auth{ID: "drain-ws-auth", Provider: "codex", Status: coreauth.StatusActive, Attributes: map[string]string{"websockets": "true"}}
	if _, err := manager.Register(context.Background(), auth); err != nil {
		t.Fatal(err)
	}
	registry.GetGlobalRegistry().RegisterClient(auth.ID, auth.Provider, []*registry.ModelInfo{{ID: "drain-model"}})
	defer registry.GetGlobalRegistry().UnregisterClient(auth.ID)
	drain := make(chan struct{})
	h := NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&sdkconfig.SDKConfig{}, manager))
	router := gin.New()
	router.GET("/v1/responses", func(c *gin.Context) {
		c.Request = c.Request.WithContext(handlers.WithGatewayDrain(c.Request.Context(), drain))
		h.ResponsesWebsocket(c)
	})
	server := httptest.NewServer(router)
	defer server.Close()
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http")+"/v1/responses", nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	request := []byte(`{"type":"response.create","model":"drain-model","input":[]}`)
	if err = conn.WriteMessage(websocket.TextMessage, request); err != nil {
		t.Fatal(err)
	}
	var active context.Context
	select {
	case active = <-executor.entered:
	case <-time.After(time.Second):
		t.Fatal("turn did not start")
	}
	if err = conn.WriteMessage(websocket.TextMessage, request); err != nil {
		t.Fatal(err)
	}
	close(drain)
	if active.Err() != nil {
		t.Fatal("drain cancelled current turn")
	}
	close(executor.release)
	_ = conn.SetReadDeadline(time.Now().Add(2 * time.Second))
	_, payload, err := conn.ReadMessage()
	if err != nil || !strings.Contains(string(payload), "response.completed") {
		t.Fatalf("completion lost: %s %v", payload, err)
	}
	_, _, err = conn.ReadMessage()
	var closed *websocket.CloseError
	if !errors.As(err, &closed) || closed.Code != websocket.CloseServiceRestart {
		t.Fatalf("expected reconnect close: %v", err)
	}
	if executor.calls.Load() != 1 {
		t.Fatal("queued request used retired account pool")
	}
}
func TestGatewayDrainWakesIdleReadPump(t *testing.T) {
	ctx, cancel := context.WithCancelCause(context.Background())
	defer cancel(nil)
	p := &responsesWebsocketReadPump{ctx: ctx, messages: make(chan responsesWebsocketInboundMessage, 1)}
	drain := make(chan struct{})
	done := make(chan error, 1)
	go func() { _, _, err := p.nextWhileServing(drain); done <- err }()
	close(drain)
	select {
	case err := <-done:
		if !errors.Is(err, errGatewayDraining) {
			t.Fatal(err)
		}
	case <-time.After(time.Second):
		t.Fatal("idle socket did not wake")
	}
}
