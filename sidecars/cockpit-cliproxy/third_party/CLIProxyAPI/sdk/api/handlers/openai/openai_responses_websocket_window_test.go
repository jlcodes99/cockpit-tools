package openai

import (
	"context"
	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	sdkconfig "github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/tidwall/gjson"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestResponsesWebsocketContextWindowBoundary(t *testing.T) {
	for _, tc := range []struct {
		name, metadata, previous string
		changed                  bool
	}{
		{"same", `{"x-codex-window-id":"w1"}`, "w1", false},
		{"first", `{"x-codex-window-id":"w1"}`, "", false},
		{"missing", `{}`, "w1", false},
		{"rollover", `{"x-codex-window-id":"w2"}`, "w1", true},
		{"metadata fallback", `{"x-codex-turn-metadata":"{\"window_id\":\"w2\"}"}`, "w1", true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			payload := []byte(`{"type":"response.create","model":"test","previous_response_id":"old","input":[{"role":"user","content":"new"}],"client_metadata":` + tc.metadata + `}`)
			got, _, changed, err := normalizeResponsesWebsocketContextWindow(payload, tc.previous)
			if err != nil || changed != tc.changed {
				t.Fatalf("boundary changed=%v err=%v", changed, err)
			}
			if gjson.GetBytes(got, "previous_response_id").Exists() == tc.changed {
				t.Fatal("incorrect continuation chain")
			}
			if gjson.GetBytes(got, "input").Raw != gjson.GetBytes(payload, "input").Raw {
				t.Fatal("window rollover changed input")
			}
		})
	}
}

func TestResponsesWebsocketWindowRolloverDoesNotReplayOldHistory(t *testing.T) {
	gin.SetMode(gin.TestMode)
	executor := &websocketCompactionCaptureExecutor{}
	manager := coreauth.NewManager(nil, nil, nil)
	manager.RegisterExecutor(executor)
	auth := &coreauth.Auth{ID: "window-followup", Provider: executor.Identifier(), Status: coreauth.StatusActive}
	if _, err := manager.Register(context.Background(), auth); err != nil {
		t.Fatal(err)
	}
	registry.GetGlobalRegistry().RegisterClient(auth.ID, auth.Provider, []*registry.ModelInfo{{ID: "window-test-model"}})
	t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(auth.ID) })
	h := NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&sdkconfig.SDKConfig{}, manager))
	router := gin.New()
	router.GET("/v1/responses/ws", h.ResponsesWebsocket)
	server := httptest.NewServer(router)
	defer server.Close()
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http")+"/v1/responses/ws", nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
	for _, request := range []string{
		`{"type":"response.create","model":"window-test-model","input":[{"type":"message","role":"user","content":"old history"}],"client_metadata":{"x-codex-window-id":"w1"}}`,
		`{"type":"response.create","model":"window-test-model","previous_response_id":"resp-1","input":[{"type":"message","role":"user","content":"new context"}],"client_metadata":{"x-codex-window-id":"w2"}}`,
	} {
		if err := conn.WriteMessage(websocket.TextMessage, []byte(request)); err != nil {
			t.Fatal(err)
		}
		_, reply, err := conn.ReadMessage()
		if err != nil {
			t.Fatal(err)
		}
		if gjson.GetBytes(reply, "type").String() != "response.completed" {
			t.Fatalf("reply=%s", reply)
		}
	}
	executor.mu.Lock()
	defer executor.mu.Unlock()
	if len(executor.streamPayloads) != 2 {
		t.Fatal("missing upstream turns")
	}
	second := executor.streamPayloads[1]
	if gjson.GetBytes(second, "previous_response_id").Exists() || strings.Contains(string(second), "old history") || gjson.GetBytes(second, "input.#").Int() != 1 {
		t.Fatalf("old context replayed into new window: %s", second)
	}
}
