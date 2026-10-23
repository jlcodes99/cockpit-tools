package executor

import (
	"context"
	"errors"
	"testing"

	"github.com/gorilla/websocket"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestCodexWebsocketSendGuardChecksAfterSocketWriteLock(t *testing.T) {
	session := &codexWebsocketSession{}
	session.writeMu.Lock()
	rejected := errors.New("quota changed behind previous writer")
	ctx := cliproxyexecutor.WithUpstreamSendGuard(context.Background(), func(_ context.Context, authID string) error {
		if authID != "selected-auth" {
			t.Fatalf("authID=%q", authID)
		}
		return rejected
	})
	done := make(chan error, 1)
	go func() {
		done <- writeCodexWebsocketMessage(ctx, "selected-auth", session, &websocket.Conn{}, []byte(`{}`))
	}()
	select {
	case <-done:
		t.Fatal("send did not wait for the write lock")
	default:
	}
	session.writeMu.Unlock()
	err := <-done
	if !errors.Is(err, rejected) || shouldRetryCodexWebsocketSend(err) {
		t.Fatalf("guard rejection retried as a disconnected socket: %v", err)
	}
	if got := mapCodexWebsocketWriteError(session, &websocket.Conn{}, err); !errors.Is(got, rejected) {
		t.Fatalf("local error was remapped: %v", got)
	}
}
