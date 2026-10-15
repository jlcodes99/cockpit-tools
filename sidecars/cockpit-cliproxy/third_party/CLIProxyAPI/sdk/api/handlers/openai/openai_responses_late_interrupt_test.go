package openai

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
)

func TestResponsesCompletedInterruptCannotCancelSuccessorOrAnotherSession(t *testing.T) {
	local := newResponsesLocalInterrupt()
	local.begin(context.Background())
	first, _ := local.interruptTurn()
	local.observeResponseID([]byte(`{"type":"response.created","response":{"id":"old"}}`))
	local.observeResponseID([]byte(`{"type":"response.completed","response":{"id":"old"}}`))
	local.end()
	old := []byte(`{"type":"response.interrupt","response_id":"old"}`)
	if !local.alreadyTerminal(old) {
		t.Fatal("idle completed response not recognized")
	}
	local.begin(context.Background())
	defer local.end()
	next, _ := local.interruptTurn()
	local.observeResponseID([]byte(`{"type":"response.created","response":{"id":"new"}}`))
	if !local.alreadyTerminal(old) || local.deliver(next, old) || local.deliver(first, old) || next.Err() != nil {
		t.Fatal("old response affected successor")
	}
	if newResponsesLocalInterrupt().alreadyTerminal(old) {
		t.Fatal("terminal ID leaked into another session")
	}
	for i := 0; i < helps.TerminalResponseIDLimit; i++ {
		local.observeResponseID([]byte(fmt.Sprintf(`{"type":"response.completed","response":{"id":"done-%d"}}`, i)))
	}
	if local.alreadyTerminal(old) {
		t.Fatal("terminal ledger failed to evict")
	}
	if local.alreadyTerminal([]byte(`{"response_id":"unknown"}`)) {
		t.Fatal("unknown response acknowledged")
	}
}

func TestResponsesLocallyInterruptedHTTPResponseIsTerminal(t *testing.T) {
	checked := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer close(checked)
		conn, err := responsesWebsocketUpgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer conn.Close()
		local := newResponsesLocalInterrupt()
		local.begin(r.Context())
		local.observeResponseID([]byte(`{"type":"response.created","response":{"id":"http-old"}}`))
		payload := []byte(`{"type":"response.interrupt","response_id":"http-old"}`)
		canceled := false
		_, _, _, _, err = completeResponsesWebsocketLocalInterrupt(newResponsesWebsocketWriter(conn), nil, func(...interface{}) { canceled = true }, payload, nil, nil, nil, local)
		if err != nil || !canceled || !local.alreadyTerminal(payload) {
			t.Errorf("synthetic HTTP terminal not recorded: err=%v canceled=%t", err, canceled)
		}
		local.begin(r.Context())
		defer local.end()
		local.observeResponseID([]byte(`{"type":"response.created","response":{"id":"http-next"}}`))
		next, _ := local.interruptTurn()
		if !local.alreadyTerminal(payload) || local.deliver(next, payload) || next.Err() != nil {
			t.Error("repeat HTTP interrupt affected the next turn")
		}
	}))
	defer server.Close()
	client, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	_ = client.SetReadDeadline(time.Now().Add(5 * time.Second))
	_, payload, err := client.ReadMessage()
	if err != nil || !strings.Contains(string(payload), `"reason":"interrupted"`) {
		t.Fatalf("missing interrupted terminal: payload=%s err=%v", payload, err)
	}
	select {
	case <-checked:
	case <-time.After(5 * time.Second):
		t.Fatal("local interruption did not finish")
	}
}
