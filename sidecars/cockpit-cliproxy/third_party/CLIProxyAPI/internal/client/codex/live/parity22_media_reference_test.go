package live

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestParity22MediaRealtimeReference(t *testing.T) {
	gin.SetMode(gin.TestMode)
	h := &Handler{sidebandAPIBaseURL: defaultSidebandAPIBaseURL}
	var urls []map[string]any
	for _, model := range []string{"gpt-realtime", "gpt-realtime-preview", "custom-live-model"} {
		urls = append(urls, map[string]any{"path": "/v1/realtime", "model": model, "url": h.directRealtimeURL(model)})
	}
	for _, path := range []string{"/v1/live/rtc_fixture", "/v1/realtime/calls/rtc_fixture", "/v1/realtime?call_id=rtc_fixture", "/backend-api/codex/rtc_fixture", "/backend-api/codex/realtime/calls/rtc_fixture"} {
		c, _ := gin.CreateTestContext(httptest.NewRecorder())
		c.Request = httptest.NewRequest(http.MethodGet, path, nil)
		if !strings.Contains(path, "?") {
			c.Params = gin.Params{{Key: "call_id", Value: "rtc_fixture"}}
		}
		style, id, ok := sidebandTarget(c)
		if !ok {
			t.Fatal(path)
		}
		urls = append(urls, map[string]any{"path": c.Request.URL.Path, "call_id": id, "model": "gpt-realtime", "url": buildSidebandURL(defaultSidebandAPIBaseURL, style, id)})
	}
	urls = append(urls, map[string]any{"path": "/v1/realtime/calls/rtc_fixture/hangup", "url": h.realtimeHTTPBaseURL() + "/realtime/calls/rtc_fixture/hangup"})

	// Exercise the actual Go WebSocket handler and its upstream/downstream
	// negotiation, using synthetic credentials and loopback sockets only.
	seen := make(chan []string, 1)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen <- websocket.Subprotocols(r)
		conn, err := (&websocket.Upgrader{Subprotocols: []string{"realtime"}, CheckOrigin: func(*http.Request) bool { return true }}).Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer conn.Close()
		_ = conn.SetReadDeadline(time.Now().Add(3 * time.Second))
		_, _, _ = conn.ReadMessage()
	}))
	defer upstream.Close()
	manager := auth.NewManager(nil, nil, nil)
	manager.RegisterExecutor(&captureExecutor{})
	registerCredential(t, manager, &auth.Auth{ID: "fixture", Provider: "codex", Status: auth.StatusActive,
		Metadata: map[string]any{"access_token": "fixture-token", "account_id": "fixture-account"}})
	handler := NewHandler(manager, nil)
	defer handler.Close()
	handler.sidebandAPIBaseURL = "ws" + strings.TrimPrefix(upstream.URL, "http") + "/v1"
	router := gin.New()
	router.GET("/v1/realtime", handler.HandleDirectWebsocket)
	proxy := httptest.NewServer(router)
	defer proxy.Close()
	dialer := websocket.Dialer{Subprotocols: []string{"realtime", "vendor.control"}, HandshakeTimeout: 3 * time.Second}
	conn, response, err := dialer.Dial("ws"+strings.TrimPrefix(proxy.URL, "http")+"/v1/realtime?model=gpt-realtime", nil)
	if response != nil && response.Body != nil {
		_ = response.Body.Close()
	}
	if err != nil {
		t.Fatal(err)
	}
	selected := conn.Subprotocol()
	_ = conn.Close()
	parity22MediaFixture(t, "parity22_realtime_go.json", map[string]any{"urls": urls, "offered_protocols": <-seen, "selected_protocol": selected})
}

func parity22MediaFixture(t *testing.T, name string, value any) {
	t.Helper()
	encoded, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	encoded = append(encoded, '\n')
	dir, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	for {
		path := filepath.Join(dir, "src-tauri", "src", "modules", "codex_native_api", "fixtures", name)
		if _, err := os.Stat(filepath.Dir(path)); err == nil {
			if os.Getenv("COCKPIT_UPDATE_PARITY22_MEDIA") == "1" {
				if err := os.WriteFile(path, encoded, 0644); err != nil {
					t.Fatal(err)
				}
			} else {
				saved, err := os.ReadFile(path)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(saved, encoded) {
					t.Fatalf("Go media fixture drift: %s; regenerate with COCKPIT_UPDATE_PARITY22_MEDIA=1", path)
				}
			}
			return
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			t.Fatal("repository fixture directory not found")
		}
		dir = parent
	}
}
