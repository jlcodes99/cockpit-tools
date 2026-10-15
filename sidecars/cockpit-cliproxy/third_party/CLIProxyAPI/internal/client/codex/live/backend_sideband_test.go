package live

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestBackendSidebandURLAndProtocol(t *testing.T) {
	for _, path := range []string{"/backend-api/codex/rtc_native", "/backend-api/codex/realtime/calls/rtc_native"} {
		c, _ := gin.CreateTestContext(httptest.NewRecorder())
		c.Request = httptest.NewRequest(http.MethodGet, path, nil)
		c.Params = gin.Params{{Key: "call_id", Value: "rtc_native"}}
		style, id, ok := sidebandTarget(c)
		if !ok || buildSidebandURL(defaultSidebandAPIBaseURL, style, id) != "wss://chatgpt.com/backend-api/codex/rtc_native" {
			t.Fatalf("native sideband target: %s style=%v id=%s valid=%v", path, style, id, ok)
		}
	}
	if got := buildSidebandURL(defaultSidebandAPIBaseURL, sidebandFrameless, "rtc_native"); got != "wss://api.openai.com/v1/live/rtc_native" {
		t.Fatalf("public Live routing changed: %s", got)
	}
}

func TestBackendSidebandPinsCallCredentialAndRelays(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/backend-api/codex/rtc_native" || r.Header.Get("Authorization") != "Bearer pinned-token" || r.Header.Get("Chatgpt-Account-Id") != "pinned-account" {
			t.Errorf("wrong native sideband binding: %s %v", r.URL.Path, r.Header)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		conn, err := (&websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}).Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer conn.Close()
		_ = conn.SetReadDeadline(time.Now().Add(3 * time.Second))
		kind, payload, err := conn.ReadMessage()
		if err == nil {
			_ = conn.WriteMessage(kind, append([]byte("echo:"), payload...))
		}
	}))
	defer upstream.Close()
	manager := auth.NewManager(nil, nil, nil)
	manager.RegisterExecutor(&captureExecutor{})
	registerCredential(t, manager, &auth.Auth{ID: "native-oauth", Provider: "codex", Status: auth.StatusActive,
		Metadata: map[string]any{"access_token": "pinned-token", "account_id": "pinned-account"}})
	handler := NewHandler(manager, nil)
	defer handler.Close()
	handler.sidebandAPIBaseURL = "ws" + strings.TrimPrefix(upstream.URL, "http") + "/backend-api/codex"
	handler.sessions.put("rtc_native", liveSession{authID: "native-oauth", model: defaultLiveModel})
	router := gin.New()
	router.GET("/backend-api/codex/:call_id", handler.HandleSideband)
	proxy := httptest.NewServer(router)
	defer proxy.Close()
	dialer := websocket.Dialer{HandshakeTimeout: 3 * time.Second}
	client, resp, err := dialer.Dial("ws"+strings.TrimPrefix(proxy.URL, "http")+"/backend-api/codex/rtc_native", nil)
	if resp != nil && resp.Body != nil {
		_ = resp.Body.Close()
	}
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	_ = client.SetReadDeadline(time.Now().Add(3 * time.Second))
	if err := client.WriteMessage(websocket.TextMessage, []byte("hello")); err != nil {
		t.Fatal(err)
	}
	_, payload, err := client.ReadMessage()
	if err != nil || string(payload) != "echo:hello" {
		t.Fatalf("native sideband relay: %s %v", payload, err)
	}
}
