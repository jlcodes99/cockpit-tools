package executor

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

func TestXAIWebsocketPongDoesNotWaitForRequestWriteLock(t *testing.T) {
	pong := make(chan struct{})
	upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.Close() }()
		conn.SetPongHandler(func(string) error { close(pong); return nil })
		if err = conn.WriteControl(websocket.PingMessage, []byte("alive"), time.Time{}); err != nil {
			return
		}
		_, _, _ = conn.ReadMessage()
	}))
	defer server.Close()
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = conn.Close() }()
	sess := &codexWebsocketSession{}
	configureXAIWebsocketConn(sess, conn)
	sess.writeMu.Lock()
	defer sess.writeMu.Unlock()
	go func() { _, _, _ = conn.ReadMessage() }()
	timer := time.NewTimer(2 * time.Second)
	defer timer.Stop()
	select {
	case <-pong:
	case <-timer.C:
		t.Fatal("pong was blocked by request writeMu")
	}
}
