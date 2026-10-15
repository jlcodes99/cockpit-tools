package proxyutil

import (
	"bufio"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"time"
)

// The server intentionally leaves handshakes or error bodies unfinished.
// Cleanup closes both ends even if a regression leaves the dial blocked.
func stalledProxy(t *testing.T, serve func(net.Conn) error) (string, <-chan error) {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	accepted := make(chan net.Conn, 1)
	done := make(chan error, 1)
	go func() {
		conn, errAccept := listener.Accept()
		if errAccept != nil {
			done <- errAccept
			return
		}
		accepted <- conn
		defer func() { _ = conn.Close() }()
		done <- serve(conn)
	}()
	t.Cleanup(func() {
		_ = listener.Close()
		select {
		case conn := <-accepted:
			_ = conn.Close()
		default:
		}
	})
	return listener.Addr().String(), done
}

func TestSOCKSTransportCancellationDuringHandshake(t *testing.T) {
	for _, scheme := range []string{"socks5", "socks5h"} {
		t.Run(scheme, func(t *testing.T) {
			greetingRead := make(chan struct{})
			addr, serverDone := stalledProxy(t, func(conn net.Conn) error {
				var greeting [3]byte
				if _, err := io.ReadFull(conn, greeting[:]); err != nil {
					return err
				}
				close(greetingRead)
				var b [1]byte
				_, err := conn.Read(b[:])
				return err
			})
			transport, _, err := BuildHTTPTransport(scheme + "://" + addr)
			if err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			dialDone := make(chan error, 1)
			go func() {
				conn, errDial := transport.DialContext(ctx, "tcp", "example.com:443")
				if conn != nil {
					_ = conn.Close()
				}
				dialDone <- errDial
			}()
			select {
			case <-greetingRead:
			case <-time.After(3 * time.Second):
				t.Fatal("SOCKS greeting was not received")
			}
			cancel()
			select {
			case errDial := <-dialDone:
				if !errors.Is(errDial, context.Canceled) {
					t.Fatalf("expected cancellation, got %v", errDial)
				}
			case <-time.After(3 * time.Second):
				t.Fatal("SOCKS handshake ignored cancellation")
			}
			select {
			case errServer := <-serverDone:
				if !errors.Is(errServer, io.EOF) {
					t.Fatalf("expected proxy connection closure, got %v", errServer)
				}
			case <-time.After(3 * time.Second):
				t.Fatal("canceled SOCKS connection remained open")
			}
		})
	}
}

func TestCONNECTFailureDoesNotDrainIncompleteBody(t *testing.T) {
	addr, serverDone := stalledProxy(t, func(conn net.Conn) error {
		if _, err := http.ReadRequest(bufio.NewReader(conn)); err != nil {
			return err
		}
		if _, err := io.WriteString(conn, "HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 100\r\n\r\n"); err != nil {
			return err
		}
		var b [1]byte
		_, err := conn.Read(b[:])
		return err
	})
	dialer, _, err := BuildDialer("http://" + addr)
	if err != nil {
		t.Fatal(err)
	}
	dialDone := make(chan error, 1)
	go func() {
		conn, errDial := dialer.Dial("tcp", "example.com:443")
		if conn != nil {
			_ = conn.Close()
		}
		dialDone <- errDial
	}()
	select {
	case errDial := <-dialDone:
		if errDial == nil || !strings.Contains(errDial.Error(), "407") {
			t.Fatalf("expected original proxy status, got %v", errDial)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("CONNECT failure blocked draining the response body")
	}
	select {
	case errServer := <-serverDone:
		if !errors.Is(errServer, io.EOF) {
			t.Fatalf("expected failed tunnel closure, got %v", errServer)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("failed tunnel remained open")
	}
}
