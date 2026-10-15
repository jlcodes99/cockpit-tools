package openai

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	"github.com/tidwall/gjson"
)

func TestResponsesLocalInterruptTracksCurrentResponse(t *testing.T) {
	local := newResponsesLocalInterrupt()
	payload := []byte(`{"type":"response.interrupt","response_id":"r1"}`)
	local.begin(context.Background())
	firstTurn, _ := local.interruptTurn()
	if local.deliver(firstTurn, payload) {
		t.Fatal("interrupt accepted before response.created")
	}
	local.observeResponseID([]byte(`{"type":"response.created","response":{"id":"r1"}}`))
	if local.deliver(firstTurn, []byte(`{"response_id":"unrelated"}`)) {
		t.Fatal("wrong response ID accepted")
	}
	if !local.deliver(firstTurn, payload) {
		t.Fatal("current response ID rejected")
	}
	local.end()
	if firstTurn.Err() == nil {
		t.Fatal("finished turn context remained active")
	}
	local.begin(context.Background())
	defer local.end()
	secondTurn, _ := local.interruptTurn()
	local.observeResponseID([]byte(`{"type":"response.created","response":{"id":"r2"}}`))
	if local.deliver(firstTurn, payload) || local.deliver(secondTurn, payload) {
		t.Fatal("old response interrupted a successor")
	}
	select {
	case <-local.framesChan():
		t.Fatal("old interrupt leaked into next turn")
	default:
	}
	if !local.deliver(secondTurn, []byte(`{"response_id":"r2"}`)) {
		t.Fatal("successor response rejected")
	}
}

func TestResponsesInterruptWorkerDoesNotBlockDisconnect(t *testing.T) {
	entered := make(chan struct{})
	canceled := make(chan struct{})
	stopped := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := responsesWebsocketUpgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		pump := newResponsesWebsocketReadPump(r.Context(), conn, func(ctx context.Context, _ []byte) error {
			close(entered)
			<-ctx.Done()
			close(canceled)
			return ctx.Err()
		}, nil)
		defer close(stopped)
		defer pump.stop()
		<-pump.ctx.Done()
	}))
	defer server.Close()
	client, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	if err := client.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.interrupt","response_id":"r1"}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case <-entered:
	case <-time.After(time.Second):
		t.Fatal("interrupt worker did not start")
	}
	_ = client.Close()
	select {
	case <-canceled:
	case <-time.After(time.Second):
		t.Fatal("blocked interrupt prevented observing downstream disconnect")
	}
	select {
	case <-stopped:
	case <-time.After(time.Second):
		t.Fatal("interrupt worker leaked after disconnect")
	}
}

func TestResponsesInterruptQueueBoundsDeduplicatesAndPreservesTurn(t *testing.T) {
	ctx, cancel := context.WithCancelCause(context.Background())
	defer cancel(context.Canceled)
	local := newResponsesLocalInterrupt()
	local.begin(ctx)
	oldTurn, _ := local.interruptTurn()
	pump := &responsesWebsocketReadPump{ctx: ctx, cancel: cancel, local: local, interruptJobs: make(chan responsesWebsocketInterruptJob, 1), interruptPending: make(map[responsesWebsocketInterruptKey]struct{})}
	payload := []byte(`{"type":"response.interrupt","response_id":"r1","extension":"unchanged"}`)
	if !pump.queueInterrupt(payload) || !pump.queueInterrupt(payload) {
		t.Fatal("duplicate interrupt rejected")
	}
	if len(pump.interruptJobs) != 1 || len(pump.interruptPending) != 1 {
		t.Fatal("duplicate interrupt occupied extra queue capacity")
	}
	local.end()
	local.begin(ctx)
	defer local.end()
	called := false
	pump.interrupt = func(context.Context, []byte) error { called = true; return nil }
	job := <-pump.interruptJobs
	if job.turn != oldTurn || job.turn.Err() == nil {
		t.Fatal("queued frame inherited the successor turn")
	}
	finished := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer close(finished)
		conn, err := responsesWebsocketUpgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer conn.Close()
		pump.writer = newResponsesWebsocketWriter(conn)
		pump.handleInterrupt(job)
	}))
	defer server.Close()
	client, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	_ = client.SetReadDeadline(time.Now().Add(time.Second))
	_, rejected, err := client.ReadMessage()
	if err != nil {
		t.Fatal(err)
	}
	<-finished
	if gjson.GetBytes(rejected, "type").String() != "error" {
		t.Fatalf("stale control error = %s", rejected)
	}
	if called {
		t.Fatal("queued old interrupt was forwarded into a successor")
	}
	secondTurn, active := local.interruptTurn()
	if ctx.Err() != nil || !active || secondTurn.Err() != nil {
		t.Fatal("old queued control canceled the connection or successor turn")
	}
}

func TestResponsesInterruptQueueOverflowCancelsWorker(t *testing.T) {
	entered := make(chan struct{})
	stopped := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := responsesWebsocketUpgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		pump := newResponsesWebsocketReadPump(r.Context(), conn, func(ctx context.Context, _ []byte) error {
			close(entered)
			<-ctx.Done()
			return ctx.Err()
		}, nil)
		defer close(stopped)
		defer pump.stop()
		<-pump.ctx.Done()
	}))
	defer server.Close()
	client, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	if err := client.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.interrupt","response_id":"r1"}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case <-entered:
	case <-time.After(time.Second):
		t.Fatal("worker did not start")
	}
	for _, id := range []string{"r2", "r3"} {
		if err := client.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.interrupt","response_id":"`+id+`"}`)); err != nil {
			t.Fatal(err)
		}
	}
	select {
	case <-stopped:
	case <-time.After(time.Second):
		t.Fatal("excess interrupt queue did not stop reader and worker")
	}
}
