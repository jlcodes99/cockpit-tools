package executor

import (
	"context"
	"errors"
	"testing"

	"github.com/gorilla/websocket"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestInterruptExecutionSessionCompletedResponseIsScopedToSocket(t *testing.T) {
	conn := &websocket.Conn{}
	sess := &codexWebsocketSession{conn: conn, authID: "one"}
	sess.resetTerminalError(conn)
	sess.terminalResponseIDs.Add("completed")
	exec := &CodexWebsocketsExecutor{store: &codexWebsocketSessionStore{sessions: map[string]*codexWebsocketSession{"session": sess}}}
	payload := []byte(`{"type":"response.interrupt","response_id":"completed"}`)
	if err := exec.InterruptExecutionSession(context.Background(), "session", payload); err != nil {
		t.Fatal(err)
	}
	if err := exec.InterruptExecutionSession(context.Background(), "other", payload); !errors.Is(err, cliproxyexecutor.ErrNoActiveUpstreamWebsocket) {
		t.Fatal("ID crossed sessions", err)
	}
	if err := exec.InterruptExecutionSession(context.Background(), "session", []byte(`{"response_id":"unknown"}`)); !errors.Is(err, cliproxyexecutor.ErrNoActiveUpstreamWebsocket) {
		t.Fatal("unknown ID acknowledged", err)
	}
	// A new account receives a new socket; an old response cannot be treated as its state.
	next := &websocket.Conn{}
	sess.conn = next
	sess.authID = "two"
	sess.resetTerminalError(next)
	if err := exec.InterruptExecutionSession(context.Background(), "session", payload); !errors.Is(err, cliproxyexecutor.ErrNoActiveUpstreamWebsocket) {
		t.Fatal("terminal ID crossed account socket", err)
	}
}

func TestInterruptExecutionSessionReaderRemembersCompletedResponse(t *testing.T) {
	server := codexWebsocketServerHoldingConnection(t, codexCreatedEvent, codexCompletedEventBody)
	defer server.Close()
	exec := NewCodexWebsocketsExecutor(codexBufferingConfig(false))
	exec.store = &codexWebsocketSessionStore{sessions: make(map[string]*codexWebsocketSession)}
	req, opts := codexWebsocketRequest()
	opts.Metadata = map[string]any{cliproxyexecutor.ExecutionSessionMetadataKey: "remember-completion"}
	result, err := exec.ExecuteStream(t.Context(), codexTestAuth(server.URL), req, opts)
	if err != nil {
		t.Fatal(err)
	}
	for chunk := range result.Chunks {
		if chunk.Err != nil {
			t.Fatal(chunk.Err)
		}
	}
	if err := exec.InterruptExecutionSession(t.Context(), "remember-completion", []byte(`{"type":"response.interrupt","response_id":"resp_1"}`)); err != nil {
		t.Fatalf("reader forgot completed response: %v", err)
	}
	exec.CloseExecutionSession("remember-completion")
}

func TestInterruptExecutionSessionCompletionWinsAuthCheckRace(t *testing.T) {
	exec, sess, _ := newInterruptTestSession(t)
	sess.resetTerminalError(sess.conn)
	captured := make(chan struct{})
	resume := make(chan struct{})
	ctx := cliproxyexecutor.WithWebsocketAuthCheck(t.Context(), func(string) bool { close(captured); <-resume; return true })
	done := make(chan error, 1)
	go func() {
		done <- exec.InterruptExecutionSession(ctx, "cancel-session", []byte(`{"type":"response.interrupt","response_id":"old"}`))
	}()
	<-captured
	sess.activeMu.Lock()
	sess.terminalResponseIDs.Add("old")
	readCh := sess.activeCh
	sess.activeMu.Unlock()
	sess.clearActive(sess.conn, readCh)
	close(resume)
	if err := <-done; err != nil {
		t.Fatalf("racing completion produced stale error: %v", err)
	}
}
