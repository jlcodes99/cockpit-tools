package main

import (
	"bytes"
	"compress/gzip"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

func TestDiagnosticSnapshotPrivacyAndBounds(t *testing.T) {
	m := &manifest{RequestPayloadLogging: true}
	d := newRequestDiagnosticsCollector(m, "privacy", time.Now())
	body := []byte(`{"messages":[{"content":"ordinary prompt may be private","metadata":{"access_token":"hidden-token"}}],"password":"hidden-password","nested":[{"apiKey":"hidden-key","arguments":"{\"refresh_token\":\"hidden-refresh\",\"text\":\"visible\"}"}]}`)
	headers := http.Header{"Content-Type": {"application/json"}, "Authorization": {"Bearer credential"}, "Cookie": {"session=credential"}, "X-Custom-Credential": {"credential"}}
	d.captureLocked("client", 0, "http", headers, body, len(body))
	pending := d.finish(context.Background(), 200)
	prepareDiagnosticSnapshots(&pending)
	encoded, _ := json.Marshal(pending.payload)
	for _, secret := range []string{"hidden-token", "hidden-password", "hidden-key", "hidden-refresh", "credential"} {
		if strings.Contains(string(encoded), secret) {
			t.Fatalf("credential leaked: %s", secret)
		}
	}
	if !strings.Contains(pending.payload.Payloads[0].Body, "ordinary prompt may be private") {
		t.Fatal("ordinary prompt text was unexpectedly changed")
	}
	if len(pending.payload.Payloads[0].SHA256) != 64 {
		t.Fatal("missing snapshot digest")
	}

	d = newRequestDiagnosticsCollector(m, "bounds", time.Now())
	for i := 0; i < 20; i++ {
		large, _ := json.Marshal(map[string]any{"content": strings.Repeat("界", 12000), "token": "hidden-token"})
		d.captureLocked("upstream", i+1, "http", headers, large, len(large))
	}
	pending = d.finish(context.Background(), 200)
	prepareDiagnosticSnapshots(&pending)
	total := 0
	for _, snapshot := range pending.payload.Payloads {
		if len(snapshot.Body) > maxDiagnosticSnapshotBytes || !utf8.ValidString(snapshot.Body) || !snapshot.Truncated || strings.Contains(snapshot.Body, "hidden-token") {
			t.Fatal("snapshot was not bounded/redacted as valid UTF-8")
		}
		total += len(snapshot.Body)
	}
	if !pending.payload.Truncated || len(pending.payload.Payloads) > maxDiagnosticPayloads || total > maxDiagnosticTotalBodyBytes {
		t.Fatal("request snapshot bound failed")
	}
}

func TestDiagnosticSnapshotCompressionAndUnsafeBodies(t *testing.T) {
	body := []byte(`{"password":"hidden","model":"test"}`)
	var compressed bytes.Buffer
	writer := gzip.NewWriter(&compressed)
	_, _ = writer.Write(body)
	_ = writer.Close()
	for _, tc := range []struct {
		name, encoding string
		body           []byte
		truncated      bool
	}{
		{"gzip", "gzip", compressed.Bytes(), false},
		{"invalid", "", []byte(`{"token":"hidden`), true},
		{"multipart", "", []byte("binary-image-secret"), true},
		{"encoding", "br", []byte("unreadable"), true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			d := newRequestDiagnosticsCollector(&manifest{RequestPayloadLogging: true}, tc.name, time.Now())
			d.captureLocked("client", 0, "http", http.Header{"Content-Encoding": {tc.encoding}}, tc.body, len(tc.body))
			pending := d.finish(context.Background(), 200)
			prepareDiagnosticSnapshots(&pending)
			snapshot := pending.payload.Payloads[0]
			if strings.Contains(snapshot.Body, "hidden") || strings.Contains(snapshot.Body, "binary-image-secret") || snapshot.Truncated != tc.truncated || snapshot.OriginalBytes != len(tc.body) {
				t.Fatalf("unsafe snapshot: %+v", snapshot)
			}
			if tc.name == "gzip" && (!strings.Contains(snapshot.Body, "REDACTED") || snapshot.Headers["content-encoding"] != "gzip") {
				t.Fatal("gzip snapshot not decoded with original encoding metadata")
			}
		})
	}
}

func TestDiagnosticsDefaultOffAndLateAttemptIsolation(t *testing.T) {
	d := newRequestDiagnosticsCollector(&manifest{}, "timing", time.Now().Add(-20*time.Millisecond))
	ctxA := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	ctxB := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	d.upstreamRequest(ctxA, cliproxyexecutor.DiagnosticRequest{AuthID: "a", Transport: "http", Body: []byte(`{"model":"one"}`)})
	d.response(ctxA, 200, http.Header{"Content-Type": {"text/event-stream"}})
	if d.firstResponseMS != nil {
		t.Fatal("HTTP headers counted as first response")
	}
	d.upstreamError(ctxA, "dial", errors.New("connection failure"))
	d.upstreamRequest(ctxB, cliproxyexecutor.DiagnosticRequest{AuthID: "b", Transport: "websocket", Body: []byte(`{"model":"two"}`)})
	d.upstreamError(ctxA, "read", errors.New("late failure"))
	d.chunk(ctxB)
	d.result(ctxB, coreauth.Result{Success: true})
	pending := d.finish(context.Background(), 200)
	prepareDiagnosticSnapshots(&pending)
	if pending.payload.FirstResponseMS == nil || *pending.payload.FirstResponseMS < 20 || len(pending.payload.Attempts) != 2 || len(pending.payload.Payloads) != 0 {
		t.Fatalf("unexpected diagnostic: %+v", pending.payload)
	}
	if pending.payload.Attempts[1].ErrorMessage != "" || !pending.payload.Attempts[1].Success || pending.payload.FailurePhase != "" {
		t.Fatal("late failure changed successor attempt")
	}
	d = newRequestDiagnosticsCollector(&manifest{}, "limit", time.Now())
	for i := 0; i < maxDiagnosticAttempts+20; i++ {
		d.upstreamRequest(context.Background(), cliproxyexecutor.DiagnosticRequest{AuthID: "a"})
	}
	if got := d.finish(context.Background(), 200).payload; len(got.Attempts) != maxDiagnosticAttempts || !got.Truncated {
		t.Fatal("attempt count is unbounded")
	}
}

func TestDiagnosticsQueueDropsWithoutBlockingRequest(t *testing.T) {
	entered, release := make(chan struct{}), make(chan struct{})
	q := newRequestDiagnosticsQueue(func(requestDiagnosticsPayload) { close(entered); <-release })
	q.submit(pendingRequestDiagnostics{})
	<-entered
	started := time.Now()
	for i := 0; i < requestDiagnosticsQueueSize+1; i++ {
		q.submit(pendingRequestDiagnostics{})
	}
	if time.Since(started) > time.Second || q.dropped.Load() != 1 {
		t.Fatal("diagnostic queue blocked request or failed to record drop")
	}
	q.close()
	close(release)
	select {
	case <-q.done:
	case <-time.After(time.Second):
		t.Fatal("diagnostic worker failed to stop")
	}
}

func TestDiagnosticsControlRequiresIndependentKeyAndDoesNotInterruptCollectors(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := &manifest{DiagnosticsControlKey: "host-only"}
	s := &relayServer{manifest: m}
	active := newRequestDiagnosticsCollector(m, "active", time.Now())
	for _, key := range []string{"", "public-api-key", "host-only"} {
		w := httptest.NewRecorder()
		c, _ := gin.CreateTestContext(w)
		c.Request = httptest.NewRequest(http.MethodPost, requestDiagnosticsConfigPath, strings.NewReader(`{"requestPayloadLogging":true}`))
		c.Request.Header.Set("Authorization", "Bearer "+key)
		s.handleRequestDiagnosticsConfig(c)
		want := 403
		if key == "host-only" {
			want = 200
		}
		if w.Code != want {
			t.Fatalf("key %q: got %d, want %d", key, w.Code, want)
		}
	}
	if active.capturePayloads || !newRequestDiagnosticsCollector(m, "new", time.Now()).capturePayloads || !m.payloadLoggingEnabled() {
		t.Fatal("control switch did not preserve active-request setting")
	}
}

func TestWebsocketDiagnosticsSharePerExecutionUsageIdentity(t *testing.T) {
	events := make(chan requestDiagnosticsPayload, 2)
	q := newRequestDiagnosticsQueue(func(payload requestDiagnosticsPayload) { events <- payload })
	defer q.close()
	o := newRequestDiagnosticsObserver(&manifest{RequestPayloadLogging: true}, "connection", time.Now(), true, q)
	ctx := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	started := time.Now()
	o.ClientPayload(ctx, []byte(`{"model":"test","input":"original"}`), http.Header{"Content-Type": {"application/json"}}, "websocket")
	o.ExecutionStarted(ctx, cliproxyexecutor.DiagnosticExecution{AuthID: "a", Model: "test", StartedAt: started})
	o.UpstreamRequest(ctx, cliproxyexecutor.DiagnosticRequest{AuthID: "a", Transport: "websocket", Body: []byte(`{"model":"test","input":"translated"}`)})
	o.UpstreamResponse(ctx, 101, nil)
	o.UpstreamChunk(ctx)
	o.recordWebsocketUsage(ctx, coreusage.Record{AuthID: "a", Model: "other-image-model", RequestedAt: started}, usagePayload{Success: true})
	if len(o.byRequestID) != 1 {
		t.Fatal("additional-model usage consumed main diagnostics")
	}
	o.recordWebsocketUsage(ctx, coreusage.Record{AuthID: "a", Model: "test", RequestedAt: started}, usagePayload{Success: true})
	select {
	case event := <-events:
		if event.RequestID != websocketUsageRequestID("connection", "a", "test", started) || event.FirstResponseMS == nil || len(event.Payloads) != 2 || len(event.Attempts) != 1 {
			t.Fatalf("wrong WebSocket execution detail: %+v", event)
		}
	case <-time.After(time.Second):
		t.Fatal("WebSocket detail missing")
	}
}

func TestDiagnosticFirstResponseTracksReadBeforeSSELine(t *testing.T) {
	d := newRequestDiagnosticsCollector(&manifest{}, "fragment", time.Now())
	o := newRequestDiagnosticsObserver(&manifest{}, "fragment", time.Now(), false, nil)
	o.base = d
	ctx := cliproxyexecutor.WithRequestDiagnosticsObserver(context.Background(), o)
	d.upstreamRequest(ctx, cliproxyexecutor.DiagnosticRequest{AuthID: "a", Transport: "http"})
	resp := &http.Response{StatusCode: 200, Body: io.NopCloser(strings.NewReader("partial SSE fragment"))}
	observeDirectGatewayResponse(ctx, resp)
	if d.firstResponseMS != nil {
		t.Fatal("headers set first response")
	}
	_, _ = resp.Body.Read(make([]byte, 2))
	if d.firstResponseMS == nil {
		t.Fatal("first body fragment was not observed")
	}
}

func TestDiagnosticEncodedEventSizeBound(t *testing.T) {
	payload := requestDiagnosticsPayload{Type: "request_diagnostics", RequestID: "encoded-bound"}
	for i := 0; i < maxDiagnosticAttempts; i++ {
		payload.Attempts = append(payload.Attempts, requestAttemptDetail{Sequence: i + 1, ErrorMessage: strings.Repeat("\\\"", 1000)})
	}
	for i := 0; i < maxDiagnosticPayloads; i++ {
		payload.Payloads = append(payload.Payloads, requestPayloadSnapshot{Body: strings.Repeat("\"\\", 8000)})
	}
	limitDiagnosticEvent(&payload)
	encoded, _ := json.Marshal(payload)
	if len(encoded) > maxDiagnosticEventBytes || !payload.Truncated {
		t.Fatalf("encoded IPC bound failed: %d", len(encoded))
	}
}

func TestDiagnosticHTTPQueueUsesControlKeyAndPollingDoesNotLogItself(t *testing.T) {
	gin.SetMode(gin.TestMode)
	queue := newRequestDiagnosticsQueue(nil)
	defer queue.close()
	for i := 0; i < 9; i++ {
		queue.ready <- requestDiagnosticsPayload{Type: "request_diagnostics", RequestID: "queued"}
	}
	m := &manifest{DiagnosticsControlKey: "host-only"}
	policy := &requestPolicy{manifest: m, emitter: &eventEmitter{}}
	policy.diagnosticsOnce.Do(func() { policy.diagnosticsQueue = queue })
	server := &relayServer{manifest: m, policy: policy}
	router := server.router()
	output := captureStdout(t, func() {
		for _, tc := range []struct {
			key   string
			count int
		}{
			{"public-key", -1}, {"host-only", 8}, {"host-only", 1}, {"host-only", 0},
		} {
			request := httptest.NewRequest(http.MethodGet, requestDiagnosticsEventsPath, nil)
			request.Header.Set("Authorization", "Bearer "+tc.key)
			w := httptest.NewRecorder()
			router.ServeHTTP(w, request)
			if tc.count == -1 {
				if w.Code != 403 || len(queue.ready) != 9 {
					t.Fatal("public key drained diagnostic queue")
				}
				continue
			}
			var result struct {
				Events []requestDiagnosticsPayload `json:"events"`
			}
			if err := json.Unmarshal(w.Body.Bytes(), &result); err != nil || w.Code != 200 || result.Events == nil || len(result.Events) != tc.count {
				t.Fatalf("unexpected batch: %d %s", w.Code, w.Body.String())
			}
		}
		request := httptest.NewRequest(http.MethodPost, requestDiagnosticsConfigPath, strings.NewReader(`{"requestPayloadLogging":true}`))
		request.Header.Set("Authorization", "Bearer host-only")
		w := httptest.NewRecorder()
		router.ServeHTTP(w, request)
		if w.Code != 200 || !m.payloadLoggingEnabled() {
			t.Fatal("control cannot update disabled public service")
		}
	})
	if output != "" || len(queue.pending) != 0 || len(queue.ready) != 0 {
		t.Fatal("diagnostic IPC emitted stdout or recursively logged polling")
	}
}
