package executor

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestCodexWebsocketPongDoesNotWaitForSessionDataWrite(t *testing.T) {
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
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	sess := &codexWebsocketSession{}
	sess.configureConn(conn)
	sess.writeMu.Lock()
	defer sess.writeMu.Unlock()
	done := make(chan error, 1)
	go func() { done <- conn.PingHandler()("liveness") }()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(time.Second):
		t.Fatal("pong blocked behind session data writer")
	}
}

func TestCodexWebsocketSOCKSHandshakeRespectsCancellation(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	accepted := make(chan net.Conn, 1)
	go func() {
		conn, err := listener.Accept()
		if err == nil {
			accepted <- conn
		}
	}()
	dialer := newProxyAwareWebsocketDialer(nil, &cliproxyauth.Auth{ProxyURL: "socks5://" + listener.Addr().String()})
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() {
		conn, err := dialer.NetDialContext(ctx, "tcp", "example.com:443")
		if conn != nil {
			conn.Close()
		}
		done <- err
	}()
	select {
	case conn := <-accepted:
		defer conn.Close()
	case <-time.After(time.Second):
		t.Fatal("proxy not contacted")
	}
	cancel()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("cancelled handshake succeeded")
		}
	case <-time.After(time.Second):
		t.Fatal("SOCKS handshake ignored cancellation")
	}
}

func TestCodexWebsocketHTTPSProxyStartsTLSHandshake(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	firstByte := make(chan byte, 1)
	go func() {
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		defer conn.Close()
		_ = conn.SetReadDeadline(time.Now().Add(time.Second))
		var buf [1]byte
		if _, err := conn.Read(buf[:]); err == nil {
			firstByte <- buf[0]
		}
	}()
	dialer := newProxyAwareWebsocketDialer(nil, &cliproxyauth.Auth{ProxyURL: "https://" + listener.Addr().String()})
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	conn, _, err := dialer.DialContext(ctx, "wss://example.com/responses", nil)
	if conn != nil {
		conn.Close()
	}
	if err == nil {
		t.Fatal("incomplete TLS proxy unexpectedly succeeded")
	}
	select {
	case b := <-firstByte:
		if b != 0x16 {
			t.Fatalf("proxy received byte %#x instead of TLS handshake", b)
		}
	case <-time.After(time.Second):
		t.Fatal("HTTPS proxy did not receive TLS handshake")
	}
}
