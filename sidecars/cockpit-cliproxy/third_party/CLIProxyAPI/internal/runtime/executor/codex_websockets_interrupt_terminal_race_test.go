package executor

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

func TestCodexWebsocketInterruptRechecksTerminalAfterWriterLock(t *testing.T) {
	accepted := make(chan *websocket.Conn, 1)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := (&websocket.Upgrader{}).Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		accepted <- conn
	}))
	defer upstream.Close()
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(upstream.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	server := <-accepted
	defer server.Close()
	if err := server.SetReadDeadline(time.Now().Add(5 * time.Second)); err != nil {
		t.Fatal(err)
	}
	sess := &codexWebsocketSession{conn: conn, authID: "writer-race-auth", wsURL: upstream.URL}
	exec := &CodexWebsocketsExecutor{}
	sess.configureConn(conn)
	readCh := sess.activate(conn)
	_, activeDone := sess.activeForConn(conn)
	readerDone := make(chan struct{})
	go func() {
		defer close(readerDone)
		exec.readUpstreamLoop(sess, conn)
	}()
	defer func() {
		_ = conn.Close()
		<-readerDone
	}()

	// Capture the current read epoch before completion. The consumer remains
	// active even after the real reader has delivered its terminal event.
	sess.writeMu.Lock()
	locked := true
	defer func() {
		if locked {
			sess.writeMu.Unlock()
		}
	}()
	started := make(chan struct{})
	finished := make(chan error, 1)
	go func() {
		close(started)
		finished <- sess.writeInterruptMessage(t.Context(), conn, readCh, activeDone, []byte(`{"type":"response.interrupt","response_id":"resp_writer_race"}`))
	}()
	<-started
	terminal := []byte(`{"type":"response.completed","response":{"id":"resp_writer_race","status":"completed","output":[]}}`)
	if err := server.WriteMessage(websocket.TextMessage, terminal); err != nil {
		t.Fatal(err)
	}
	select {
	case event := <-readCh:
		if event.err != nil || !bytes.Equal(event.payload, terminal) {
			t.Fatalf("terminal read = %s, error = %v", event.payload, event.err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("reader did not deliver completion")
	}
	sess.writeMu.Unlock()
	locked = false
	select {
	case err := <-finished:
		if err != nil {
			t.Fatalf("completed interrupt should be a no-op: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("interrupt did not finish after writer was released")
	}

	// A following accepted frame is a deterministic barrier: the completed
	// interrupt must not have been written ahead of it.
	barrier := []byte(`{"type":"response.create","model":"barrier","input":[]}`)
	if err := conn.WriteMessage(websocket.TextMessage, barrier); err != nil {
		t.Fatal(err)
	}
	_, payload, err := server.ReadMessage()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(payload, barrier) {
		t.Fatalf("completed interrupt was written upstream: %s", payload)
	}
}
