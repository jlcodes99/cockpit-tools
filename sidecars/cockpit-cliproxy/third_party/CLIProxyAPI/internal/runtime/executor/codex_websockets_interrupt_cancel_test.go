package executor

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// The transport gates only post-handshake writes and releases them on Close,
// reproducing socket backpressure without relying on OS buffer sizes or sleeps.
type interruptBlockedConn struct {
	net.Conn
	block       atomic.Bool
	started     chan struct{}
	closed      chan struct{}
	startedOnce sync.Once
	closeOnce   sync.Once
}

func (c *interruptBlockedConn) Write(payload []byte) (int, error) {
	if c.block.Load() {
		c.startedOnce.Do(func() { close(c.started) })
		<-c.closed
		return 0, net.ErrClosed
	}
	return c.Conn.Write(payload)
}

func (c *interruptBlockedConn) Close() error {
	c.closeOnce.Do(func() { close(c.closed) })
	return c.Conn.Close()
}

func newInterruptTestSession(t *testing.T) (*CodexWebsocketsExecutor, *codexWebsocketSession, *interruptBlockedConn) {
	t.Helper()
	accepted := make(chan *websocket.Conn, 1)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := (&websocket.Upgrader{}).Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		accepted <- conn
	}))
	t.Cleanup(upstream.Close)
	var transport *interruptBlockedConn
	dialer := websocket.Dialer{NetDialContext: func(ctx context.Context, network, addr string) (net.Conn, error) {
		conn, err := (&net.Dialer{}).DialContext(ctx, network, addr)
		if err != nil {
			return nil, err
		}
		transport = &interruptBlockedConn{Conn: conn, started: make(chan struct{}), closed: make(chan struct{})}
		return transport, nil
	}}
	client, _, err := dialer.Dial("ws"+strings.TrimPrefix(upstream.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = client.Close() })
	server := <-accepted
	t.Cleanup(func() { _ = server.Close() })
	sess := &codexWebsocketSession{conn: client, authID: "interrupt-cancel-auth", wsURL: upstream.URL}
	sess.activate(client)
	executor := &CodexWebsocketsExecutor{store: &codexWebsocketSessionStore{sessions: map[string]*codexWebsocketSession{"cancel-session": sess}}}
	return executor, sess, transport
}

func TestInterruptExecutionSessionCancelsBlockedSocketWrite(t *testing.T) {
	executor, _, transport := newInterruptTestSession(t)
	transport.block.Store(true)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() {
		done <- executor.InterruptExecutionSession(ctx, "cancel-session", []byte(`{"type":"response.interrupt","response_id":"r1"}`))
	}()
	select {
	case <-transport.started:
	case <-time.After(time.Second):
		t.Fatal("interrupt did not enter socket write")
	}
	cancel()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("write cancellation = %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("context cancellation did not release blocked interrupt writer")
	}
}

func TestInterruptExecutionSessionCancelsWaitingForWriter(t *testing.T) {
	executor, sess, transport := newInterruptTestSession(t)
	sess.writeMu.Lock()
	defer sess.writeMu.Unlock()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	// The auth check establishes that the session was captured before cancel.
	captured := make(chan struct{})
	ctx = cliproxyexecutor.WithWebsocketAuthCheck(ctx, func(string) bool { close(captured); return true })
	go func() {
		done <- executor.InterruptExecutionSession(ctx, "cancel-session", []byte(`{"type":"response.interrupt","response_id":"r1"}`))
	}()
	<-captured
	cancel()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("lock cancellation = %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("interrupt waited indefinitely for writeMu")
	}
	select {
	case <-transport.closed:
		t.Fatal("cancelled lock wait closed an unwritten socket")
	default:
	}
}

func TestInterruptExecutionSessionDoesNotWriteIntoSuccessorReadEpoch(t *testing.T) {
	executor, sess, transport := newInterruptTestSession(t)
	transport.block.Store(true)
	sess.writeMu.Lock()
	captured := make(chan struct{})
	resume := make(chan struct{})
	ctx := cliproxyexecutor.WithWebsocketAuthCheck(context.Background(), func(string) bool { close(captured); <-resume; return true })
	done := make(chan error, 1)
	go func() {
		done <- executor.InterruptExecutionSession(ctx, "cancel-session", []byte(`{"type":"response.interrupt","response_id":"old"}`))
	}()
	<-captured
	// Reuse the same physical socket for a new execution epoch before the old
	// control frame obtains its write lock.
	sess.activate(sess.conn)
	close(resume)
	sess.writeMu.Unlock()
	select {
	case err := <-done:
		if !errors.Is(err, cliproxyexecutor.ErrNoActiveUpstreamWebsocket) {
			t.Fatalf("stale interrupt result = %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("old epoch control did not return")
	}
	select {
	case <-transport.closed:
		t.Fatal("old control closed successor socket")
	default:
	}
}

func TestInterruptExecutionSessionCancellationClosesOnlyCapturedSocket(t *testing.T) {
	executor, sess, transport := newInterruptTestSession(t)
	_, successor, successorTransport := newInterruptTestSession(t)
	transport.block.Store(true)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() {
		done <- executor.InterruptExecutionSession(ctx, "cancel-session", []byte(`{"type":"response.interrupt","response_id":"old"}`))
	}()
	select {
	case <-transport.started:
	case <-time.After(time.Second):
		t.Fatal("write did not start")
	}
	sess.connMu.Lock()
	sess.conn = successor.conn
	sess.connMu.Unlock()
	sess.activate(successor.conn)
	cancel()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("cancellation result = %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("captured write did not finish after cancellation")
	}
	select {
	case <-successorTransport.closed:
		t.Fatal("canceling old write closed successor connection")
	default:
	}
}
