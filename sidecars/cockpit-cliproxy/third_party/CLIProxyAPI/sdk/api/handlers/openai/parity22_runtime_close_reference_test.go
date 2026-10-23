package openai

import (
	"encoding/json"
	"errors"
	"net/http/httptest"
	"os"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	sdkconfig "github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

type parity22RuntimeStatusError struct {
	status int
	body   string
}

func (e parity22RuntimeStatusError) Error() string   { return e.body }
func (e parity22RuntimeStatusError) StatusCode() int { return e.status }

// Capture real downstream close frames through the SDK's Responses handler.
func TestParity22RuntimeCloseReference(t *testing.T) {
	gin.SetMode(gin.TestMode)
	actual := make([]map[string]any, 0)
	for _, tc := range []struct {
		name string
		err  error
	}{
		{"upstream_1009", &websocket.CloseError{Code: websocket.CloseMessageTooBig, Text: "message too big"}},
		{"structured_413", parity22RuntimeStatusError{413, `{"error":{"code":"message_too_big","message":"fixture frame too large"}}`}},
	} {
		executor := &websocketUpstreamDisconnectExecutor{provider: "codex", subscribed: make(chan string, 1)}
		manager := coreauth.NewManager(nil, nil, nil)
		manager.RegisterExecutor(executor)
		h := NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&sdkconfig.SDKConfig{}, manager))
		router := gin.New()
		router.GET("/v1/responses", h.ResponsesWebsocket)
		server := httptest.NewServer(router)
		conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http")+"/v1/responses", nil)
		if err != nil {
			server.Close()
			t.Fatal(err)
		}
		select {
		case session := <-executor.subscribed:
			executor.TriggerDisconnect(session, tc.err)
		case <-time.After(time.Second):
			t.Fatal("missing upstream disconnect subscription")
		}
		_ = conn.SetReadDeadline(time.Now().Add(time.Second))
		_, _, err = conn.ReadMessage()
		var closeErr *websocket.CloseError
		if !errors.As(err, &closeErr) {
			t.Fatalf("%s: expected close frame, got %v", tc.name, err)
		}
		actual = append(actual, map[string]any{"name": tc.name, "code": closeErr.Code, "reason": closeErr.Text})
		_ = conn.Close()
		server.Close()
	}
	if matched, _ := websocketClosePayloadForUpstreamError(parity22RuntimeStatusError{413, `{"error":{"code":"request_too_large"}}`}); matched {
		t.Fatal("generic 413 must not become 1009")
	}
	data, err := json.MarshalIndent(actual, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	path := os.Getenv("PARITY22_RUNTIME_CLOSE_REFERENCE_OUT")
	if path != "" {
		if err := os.WriteFile(path, append(data, '\n'), 0600); err != nil {
			t.Fatal(err)
		}
		return
	}
	want, err := os.ReadFile("../../../../../../../../src-tauri/src/modules/codex_native_api/fixtures/parity22_runtime_close_go.json")
	if err != nil {
		t.Fatal(err)
	}
	var gotJSON, wantJSON any
	_ = json.Unmarshal(data, &gotJSON)
	if err := json.Unmarshal(want, &wantJSON); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(gotJSON, wantJSON) {
		t.Fatalf("runtime close reference changed: %s", data)
	}
}
