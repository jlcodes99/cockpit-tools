package openai

import (
	"context"
	"net/http/httptest"
	"strings"
	"sync"
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
	"github.com/tidwall/gjson"
)

type tierResponsesWebsocketExecutor struct {
	homeResponsesWebsocketExecutor
	tierMu sync.Mutex
	tiers  []string
}

func (e *tierResponsesWebsocketExecutor) ExecuteStream(ctx context.Context, auth *coreauth.Auth, req coreexecutor.Request, opts coreexecutor.Options) (*coreexecutor.StreamResult, error) {
	tier, _ := coreexecutor.ServiceTierDefaultSnapshot(ctx)
	e.tierMu.Lock()
	e.tiers = append(e.tiers, tier)
	e.tierMu.Unlock()
	return e.homeResponsesWebsocketExecutor.ExecuteStream(ctx, auth, req, opts)
}

func TestResponsesWebsocketServiceTierDefaultChangesOnSameConnection(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var current atomic.Value
	current.Store("")
	exec := &tierResponsesWebsocketExecutor{}
	manager := coreauth.NewManager(nil, nil, nil)
	manager.RegisterExecutor(exec)
	const authID = "ws-tier-default-auth"
	const model = "ws-tier-default-model"
	if _, err := manager.Register(context.Background(), &coreauth.Auth{ID: authID, Provider: "codex", Status: coreauth.StatusActive}); err != nil {
		t.Fatal(err)
	}
	registry.GetGlobalRegistry().RegisterClient(authID, "codex", []*registry.ModelInfo{{ID: model}})
	t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(authID) })
	h := NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&sdkconfig.SDKConfig{}, manager))
	router := gin.New()
	router.Use(func(c *gin.Context) {
		ctx := coreexecutor.WithServiceTierDefaultProvider(c.Request.Context(), func() string { return current.Load().(string) })
		c.Request = c.Request.WithContext(ctx)
		c.Next()
	})
	router.GET("/v1/responses", h.ResponsesWebsocket)
	server := httptest.NewServer(router)
	defer server.Close()
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http")+"/v1/responses", nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	for _, tier := range []string{"", "priority", ""} {
		current.Store(tier)
		if err := conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.create","model":"ws-tier-default-model","input":[]}`)); err != nil {
			t.Fatal(err)
		}
		if err := conn.SetReadDeadline(time.Now().Add(5 * time.Second)); err != nil {
			t.Fatal(err)
		}
		for {
			_, body, err := conn.ReadMessage()
			if err != nil {
				t.Fatal(err)
			}
			event := gjson.GetBytes(body, "type").String()
			if event == "error" {
				t.Fatalf("websocket error: %s", body)
			}
			if event == "response.completed" {
				break
			}
		}
	}
	exec.tierMu.Lock()
	defer exec.tierMu.Unlock()
	if len(exec.tiers) != 3 || exec.tiers[0] != "" || exec.tiers[1] != "priority" || exec.tiers[2] != "" {
		t.Fatalf("turn tiers = %#v", exec.tiers)
	}
}
