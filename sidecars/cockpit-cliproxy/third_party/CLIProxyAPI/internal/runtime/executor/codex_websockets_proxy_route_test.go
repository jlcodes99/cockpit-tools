package executor

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

func TestCodexWebsocketProxyRouteSurvivesSessionReuseAndConfigurationChange(t *testing.T) {
	upgrader := websocket.Upgrader{}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		defer conn.Close()
		for {
			if _, _, err := conn.ReadMessage(); err != nil {
				return
			}
		}
	}))
	defer server.Close()
	observed := 0
	helps.SetRequestProxyRouteObserver(func(proxyURL string, conn net.Conn) func() *usage.ProxyRoute {
		observed++
		if proxyURL != "" || conn == nil {
			t.Fatalf("actual route = %q, connection nil = %v", proxyURL, conn == nil)
		}
		return func() *usage.ProxyRoute { return &usage.ProxyRoute{Kind: "direct"} }
	})
	defer helps.SetRequestProxyRouteObserver(nil)
	executor := NewCodexWebsocketsExecutor(&config.Config{})
	session := &codexWebsocketSession{sessionID: "proxy-route-test"}
	auth := &cliproxyauth.Auth{ID: "test", ProxyURL: "direct"}
	wsURL := "ws" + strings.TrimPrefix(server.URL, "http")
	conn, closer, _, err := executor.ensureUpstreamConn(context.Background(), auth, session, auth.ID, wsURL, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer closer.Close()
	reused, reusedCloser, _, err := executor.ensureUpstreamConn(context.Background(), auth, session, auth.ID, wsURL, nil)
	if err != nil {
		t.Fatal(err)
	}
	if conn != reused || closer != reusedCloser || observed != 1 {
		t.Fatalf("connection not reused or observation replaced: observations=%d", observed)
	}
	if route := reusedCloser.proxyRouteGetter(); route == nil || route.Kind != "direct" {
		t.Fatalf("original connection route = %+v", route)
	}
	auth.ProxyURL = "http://127.0.0.1:1"
	if got, _ := existingWebsocketSessionConn(session, auth.ID, wsURL, codexWebsocketProxyURL(executor.cfg, auth)); got != nil {
		t.Fatal("connection through previous proxy reused")
	}
	if _, _, _, err := executor.ensureUpstreamConn(context.Background(), auth, session, auth.ID, wsURL, nil); err == nil {
		t.Fatal("expected redial through changed proxy to fail")
	}
	if route := closer.proxyRouteGetter(); route == nil || route.Kind != "direct" {
		t.Fatalf("original connection route lost after proxy change: %+v", route)
	}
}
