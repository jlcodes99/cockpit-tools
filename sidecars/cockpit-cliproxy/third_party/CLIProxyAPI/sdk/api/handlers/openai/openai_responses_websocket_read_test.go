package openai

import (
	"bytes"
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

func TestResponsesWebsocketReadPumpKeepsFIFOAndStopsOnRealClosure(t *testing.T) {
	result := make(chan []string, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := responsesWebsocketUpgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		pump := newResponsesWebsocketReadPump(r.Context(), conn)
		defer pump.stop()
		var received []string
		for range 3 {
			_, payload, errNext := pump.next()
			if errNext != nil {
				t.Error(errNext)
				return
			}
			received = append(received, string(payload))
		}
		result <- received
		select {
		case <-pump.ctx.Done():
		case <-time.After(time.Second):
			t.Error("real client closure was not observed while handler was idle")
		}
	}))
	defer server.Close()
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	for _, value := range []string{"one", "two", "three"} {
		if errWrite := conn.WriteMessage(websocket.TextMessage, []byte(value)); errWrite != nil {
			t.Fatal(errWrite)
		}
	}
	select {
	case got := <-result:
		if strings.Join(got, ",") != "one,two,three" {
			t.Fatalf("message order changed: %v", got)
		}
	case <-time.After(time.Second):
		t.Fatal("queued messages were not delivered")
	}
	_ = conn.Close()
}

func TestResponsesWebsocketReadPumpBoundsPendingFrames(t *testing.T) {
	pumps := make(chan *responsesWebsocketReadPump, 1)
	stopped := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := responsesWebsocketUpgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		pump := newResponsesWebsocketReadPump(r.Context(), conn)
		defer func() {
			pump.stop()
			close(stopped)
		}()
		pumps <- pump
		<-pump.done
	}))
	defer server.Close()
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	pump := <-pumps
	for range responsesWebsocketInboundQueueSize + 1 {
		_ = conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.create"}`))
	}
	select {
	case <-pump.ctx.Done():
		if !strings.Contains(context.Cause(pump.ctx).Error(), "queue full") || len(pump.messages) > responsesWebsocketInboundQueueSize {
			t.Fatal("pending frame overflow did not cancel bounded connection")
		}
	case <-time.After(time.Second):
		t.Fatal("inbound queue overflow blocked disconnect detection")
	}
	_, _, errRead := conn.ReadMessage()
	var closeErr *websocket.CloseError
	if !errors.As(errRead, &closeErr) || closeErr.Code != websocket.ClosePolicyViolation || !strings.Contains(closeErr.Text, "queue full") {
		t.Fatalf("queue overflow did not send a protocol close reason: %v", errRead)
	}
	select {
	case <-stopped:
		if len(pump.messages) != 0 || pump.bytes.Load() != 0 {
			t.Fatal("queued frame memory remained after pump stop")
		}
	case <-time.After(time.Second):
		t.Fatal("queue overflow left a reader goroutine running")
	}
}

func TestResponsesWebsocketReadPumpPreservesSingleLargeFrame(t *testing.T) {
	sizes := make(chan int, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := responsesWebsocketUpgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		pump := newResponsesWebsocketReadPump(r.Context(), conn)
		defer pump.stop()
		_, payload, errNext := pump.next()
		if errNext != nil {
			t.Error(errNext)
			return
		}
		sizes <- len(payload)
	}))
	defer server.Close()
	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	payload := bytes.Repeat([]byte("x"), responsesWebsocketInboundMaxBytes+1)
	if errWrite := conn.WriteMessage(websocket.TextMessage, payload); errWrite != nil {
		t.Fatal(errWrite)
	}
	select {
	case size := <-sizes:
		if size != len(payload) {
			t.Fatal("existing legal single frame was truncated")
		}
	case <-time.After(3 * time.Second):
		t.Fatal("single large frame was rejected by pending-message budget")
	}
}
