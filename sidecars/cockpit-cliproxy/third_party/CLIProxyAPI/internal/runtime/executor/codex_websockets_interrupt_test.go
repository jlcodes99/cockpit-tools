package executor

import (
	"bytes"
	"context"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestInterruptExecutionSessionWithoutUpstream(t *testing.T) {
	store := &codexWebsocketSessionStore{sessions: map[string]*codexWebsocketSession{"disconnected": {}}}
	executor := &CodexWebsocketsExecutor{store: store}
	var nilWebsocket *CodexWebsocketsExecutor
	var nilAuto *CodexAutoExecutor
	for _, tc := range []struct {
		name      string
		interrupt func(context.Context, string, []byte) error
		sessionID string
	}{
		{"nil websocket executor", nilWebsocket.InterruptExecutionSession, "missing"},
		{"nil auto executor", nilAuto.InterruptExecutionSession, "missing"},
		{"auto without websocket executor", (&CodexAutoExecutor{}).InterruptExecutionSession, "missing"},
		{"empty session", executor.InterruptExecutionSession, ""},
		{"blank session", executor.InterruptExecutionSession, " \t "},
		{"missing session", executor.InterruptExecutionSession, "missing"},
		{"disconnected session", executor.InterruptExecutionSession, "disconnected"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			err := tc.interrupt(context.Background(), tc.sessionID, []byte(`{"type":"response.interrupt","response_id":"r1"}`))
			if !errors.Is(err, cliproxyexecutor.ErrNoActiveUpstreamWebsocket) {
				t.Fatalf("error = %v, want ErrNoActiveUpstreamWebsocket", err)
			}
			if len(store.sessions) != 1 || store.sessions["disconnected"] == nil {
				t.Fatal("interrupt changed the session store")
			}
		})
	}
}

// Adapted from upstream 6331b38; read the next accepted frame to detect rejected
// writes without a timing-based negative assertion.
func TestInterruptExecutionSessionRequiresActiveRead(t *testing.T) {
	accepted := make(chan *websocket.Conn, 1)
	var connections atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		connections.Add(1)
		conn, err := (&websocket.Upgrader{}).Upgrade(w, r, nil)
		if err != nil {
			t.Errorf("upgrade: %v", err)
			return
		}
		select {
		case accepted <- conn:
		default:
			_ = conn.Close()
		}
	}))
	defer upstream.Close()
	client, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(upstream.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	var server *websocket.Conn
	select {
	case server = <-accepted:
	case <-time.After(5 * time.Second):
		t.Fatal("upstream did not accept the connection")
	}
	defer server.Close()
	if err := server.SetReadDeadline(time.Now().Add(5 * time.Second)); err != nil {
		t.Fatal(err)
	}
	if err := client.SetWriteDeadline(time.Now().Add(5 * time.Second)); err != nil {
		t.Fatal(err)
	}

	const sessionID = "interrupt-requires-active-read"
	sess := &codexWebsocketSession{conn: client, authID: "auth", wsURL: "ws" + strings.TrimPrefix(upstream.URL, "http")}
	executor := &CodexWebsocketsExecutor{store: &codexWebsocketSessionStore{sessions: map[string]*codexWebsocketSession{sessionID: sess}}}
	auto := &CodexAutoExecutor{wsExec: executor}
	rejected := []byte(`{"type":"response.interrupt","response_id":"rejected"}`)

	if err := auto.InterruptExecutionSession(context.Background(), sessionID, rejected); !errors.Is(err, cliproxyexecutor.ErrNoActiveUpstreamWebsocket) {
		t.Fatalf("idle socket error = %v, want ErrNoActiveUpstreamWebsocket", err)
	}
	sess.setActive(&websocket.Conn{}, make(chan codexWebsocketRead))
	if err := auto.InterruptExecutionSession(context.Background(), sessionID, rejected); !errors.Is(err, cliproxyexecutor.ErrNoActiveUpstreamWebsocket) {
		t.Fatalf("old connection error = %v, want ErrNoActiveUpstreamWebsocket", err)
	}
	readCh := sess.activate(client)
	defer sess.clearActive(client, readCh)
	canceled, cancel := context.WithCancel(context.Background())
	cancel()
	if err := auto.InterruptExecutionSession(canceled, sessionID, rejected); !errors.Is(err, context.Canceled) {
		t.Fatalf("canceled context error = %v, want context.Canceled", err)
	}
	var checkedAuth string
	disabled := cliproxyexecutor.WithWebsocketAuthCheck(context.Background(), func(authID string) bool {
		checkedAuth = authID
		return false
	})
	if err := auto.InterruptExecutionSession(disabled, sessionID, rejected); err == nil || !strings.Contains(err.Error(), "credential is no longer enabled") {
		t.Fatalf("disabled credential error = %v", err)
	}
	if checkedAuth != "auth" {
		t.Fatalf("checked credential = %q, want auth", checkedAuth)
	}

	interrupt := []byte(`{ "type": "response.interrupt", "response_id": "r1", "mode": "discard_partial_items", "extension": {"value": [1, true, "原样"]} }`)
	for _, ctx := range []context.Context{nil, context.Background()} {
		if err := auto.InterruptExecutionSession(ctx, " "+sessionID+" ", interrupt); err != nil {
			t.Fatal(err)
		}
		messageType, payload, err := server.ReadMessage()
		if err != nil {
			t.Fatal(err)
		}
		if messageType != websocket.TextMessage || !bytes.Equal(payload, interrupt) {
			t.Fatalf("forwarded frame = (%d, %s), want original interrupt", messageType, payload)
		}
	}

	sess.clearActive(client, readCh)
	if err := auto.InterruptExecutionSession(context.Background(), sessionID, rejected); !errors.Is(err, cliproxyexecutor.ErrNoActiveUpstreamWebsocket) {
		t.Fatalf("late interrupt error = %v, want ErrNoActiveUpstreamWebsocket", err)
	}
	readCh = sess.activate(client)
	defer sess.clearActive(client, readCh)
	if err := client.Close(); err != nil {
		t.Fatal(err)
	}
	if err := auto.InterruptExecutionSession(context.Background(), sessionID, interrupt); !errors.Is(err, net.ErrClosed) {
		t.Fatalf("closed socket error = %v, want net.ErrClosed", err)
	}
	if _, payload, err := server.ReadMessage(); err == nil {
		t.Fatalf("unexpected extra frame: %s", payload)
	} else if networkErr, ok := err.(net.Error); ok && networkErr.Timeout() {
		t.Fatalf("expected closed connection, got timeout: %v", err)
	}
	if connections.Load() != 1 {
		t.Fatalf("connection count = %d, want 1", connections.Load())
	}
	if len(executor.store.sessions) != 1 || executor.store.sessions[sessionID] != sess || sess.conn != client || sess.authID != "auth" {
		t.Fatal("interrupt replaced the session, connection, or credential")
	}
}
