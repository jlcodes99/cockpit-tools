package main

import (
	"context"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
)

func TestGatewayDrainReleasesListenerBeforeAcceptedResponseFinishes(t *testing.T) {
	drain := newGatewayDrain()
	entered, release := make(chan struct{}), make(chan struct{})
	handler := drain.wrap(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if handlers.GatewayDrain(r.Context()) == nil {
			t.Error("missing retirement signal")
		}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "data: first\n\n")
		w.(http.Flusher).Flush()
		close(entered)
		<-release
		if r.Context().Err() != nil {
			t.Error("retirement cancelled accepted request")
		}
		_, _ = io.WriteString(w, "data: completed\n\n")
	}))
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	server := &http.Server{Handler: handler}
	go server.Serve(listener)
	defer server.Close()
	client := &http.Client{Timeout: 3 * time.Second}
	response, err := client.Get("http://" + listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	<-entered
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	finished := make(chan error, 1)
	go func() { finished <- drain.finish(ctx, server) }()
	var replacement net.Listener
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		replacement, err = net.Listen("tcp", listener.Addr().String())
		if err == nil {
			break
		}
		time.Sleep(time.Millisecond)
	}
	if err != nil {
		close(release)
		t.Fatal("old listener did not release port", err)
	}
	replacementServer := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { io.WriteString(w, "new-account-pool") })}
	go replacementServer.Serve(replacement)
	defer replacementServer.Close()
	next, err := client.Get("http://" + replacement.Addr().String())
	if err != nil {
		close(release)
		t.Fatal(err)
	}
	data, _ := io.ReadAll(next.Body)
	next.Body.Close()
	if string(data) != "new-account-pool" {
		t.Fatalf("new request used old listener: %s", data)
	}
	select {
	case err := <-finished:
		close(release)
		t.Fatalf("retired runtime stopped early: %v", err)
	default:
	}
	close(release)
	data, err = io.ReadAll(response.Body)
	if err != nil || !strings.Contains(string(data), "completed") {
		t.Fatalf("accepted stream lost completion: %s %v", data, err)
	}
	if err = <-finished; err != nil {
		t.Fatal(err)
	}
}

func TestGatewayDrainControlAndAdmission(t *testing.T) {
	for _, tc := range []struct {
		input string
		drain bool
	}{{"drain\n", true}, {"", false}, {"unknown\n", false}, {strings.Repeat("x", 2048), false}} {
		d := newGatewayDrain()
		parentClosed := false
		d.control(strings.NewReader(tc.input), func() { parentClosed = true })
		if parentClosed == tc.drain {
			t.Fatalf("unexpected parent status for %q", tc.input)
		}
		if tc.drain {
			d.request()
			w := httptest.NewRecorder()
			d.wrap(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { t.Error("admitted after drain") })).ServeHTTP(w, httptest.NewRequest("GET", "/", nil))
			if w.Code != 503 || w.Header().Get("Retry-After") != "1" {
				t.Fatalf("missing retryable rejection: %+v", w)
			}
		}
	}
}

func TestRetiringDiagnosticsFlushReadyAndPendingExactlyOnce(t *testing.T) {
	received := make(chan requestDiagnosticsPayload, 10)
	q := newRequestDiagnosticsQueue(nil, func(p requestDiagnosticsPayload) { received <- p })
	q.submit(pendingRequestDiagnostics{payload: requestDiagnosticsPayload{RequestID: "ready"}})
	deadline := time.Now().Add(time.Second)
	for len(q.ready) == 0 && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if len(q.ready) != 1 {
		t.Fatal("diagnostics not prepared")
	}
	q.push.Store(true)
	q.wake <- struct{}{}
	q.submit(pendingRequestDiagnostics{payload: requestDiagnosticsPayload{RequestID: "pending"}})
	q.close()
	select {
	case <-q.done:
	case <-time.After(time.Second):
		t.Fatal("retiring diagnostics failed to flush")
	}
	if len(received) != 2 {
		t.Fatalf("lost/duplicated diagnostic: %d", len(received))
	}
	seen := map[string]bool{}
	for len(received) > 0 {
		p := <-received
		if seen[p.RequestID] {
			t.Fatal("duplicate", p.RequestID)
		}
		seen[p.RequestID] = true
	}
}

func TestGatewayDrainDeadlineDoesNotWaitForever(t *testing.T) {
	d := newGatewayDrain()
	release, entered := make(chan struct{}), make(chan struct{})
	server := httptest.NewUnstartedServer(d.wrap(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { close(entered); <-release })))
	server.Start()
	requestDone := make(chan struct{})
	go func() {
		defer close(requestDone)
		response, _ := server.Client().Get(server.URL)
		if response != nil {
			response.Body.Close()
		}
	}()
	<-entered
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	err := d.finish(ctx, server.Config)
	close(release)
	<-requestDone
	server.Close()
	if err != context.DeadlineExceeded {
		t.Fatalf("expected bounded shutdown, got %v", err)
	}
}
