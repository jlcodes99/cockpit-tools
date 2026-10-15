package main

import (
	"context"
	"io"
	"net/http"
	"sync"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

// HTTP calls have one collector. WebSocket usage is already emitted per upstream
// execution; the observer uses that same execution identity for request details.
type requestDiagnosticsObserver struct {
	mu           sync.Mutex
	base         *requestDiagnosticsCollector
	websocket    bool
	queue        *requestDiagnosticsQueue
	byAttempt    map[any]*requestDiagnosticsCollector
	byRequestID  map[string]*requestDiagnosticsCollector
	clientBody   []byte
	clientBytes  int
	clientHeader http.Header
}

func newRequestDiagnosticsObserver(m *manifest, id string, started time.Time, websocket bool, queue *requestDiagnosticsQueue) *requestDiagnosticsObserver {
	return &requestDiagnosticsObserver{
		base: newRequestDiagnosticsCollector(m, id, started), websocket: websocket, queue: queue,
		byAttempt: make(map[any]*requestDiagnosticsCollector), byRequestID: make(map[string]*requestDiagnosticsCollector),
	}
}

func requestDiagnosticsFromContext(ctx context.Context) *requestDiagnosticsObserver {
	observer, _ := cliproxyexecutor.DiagnosticsObserver(ctx).(*requestDiagnosticsObserver)
	return observer
}

func (o *requestDiagnosticsObserver) collector(ctx context.Context) *requestDiagnosticsCollector {
	if !o.websocket {
		return o.base
	}
	o.mu.Lock()
	defer o.mu.Unlock()
	return o.byAttempt[cliproxyexecutor.UpstreamAttemptIdentity(ctx)]
}

func (o *requestDiagnosticsObserver) ClientPayload(ctx context.Context, body []byte, headers http.Header, transport string) {
	o.captureClient(ctx, body, headers, transport, len(body))
}

func (o *requestDiagnosticsObserver) captureClient(_ context.Context, body []byte, headers http.Header, transport string, originalBytes int) {
	if !o.websocket {
		o.base.mu.Lock()
		defer o.base.mu.Unlock()
		o.base.captureLocked("client", 0, transport, headers, body, originalBytes)
		return
	}
	if !o.base.manifest.payloadLoggingEnabled() {
		return
	}
	o.mu.Lock()
	defer o.mu.Unlock()
	o.clientBytes = originalBytes
	o.clientBody = nil
	if len(body) <= maxDiagnosticRawBodyBytes {
		o.clientBody = append([]byte(nil), body...)
	}
	o.clientHeader = headers.Clone()
}

func (o *requestDiagnosticsObserver) ExecutionStarted(ctx context.Context, info cliproxyexecutor.DiagnosticExecution) {
	identity := cliproxyexecutor.UpstreamAttemptIdentity(ctx)
	if !o.websocket {
		o.base.mu.Lock()
		defer o.base.mu.Unlock()
		if !o.base.closed && len(o.base.executions) < maxDiagnosticAttempts {
			o.base.executions[identity] = info
		}
		return
	}
	o.mu.Lock()
	defer o.mu.Unlock()
	if len(o.byRequestID) >= maxDiagnosticAttempts {
		// This is a bound on concurrent/pending callbacks, not on the lifetime
		// number of turns in a WebSocket connection.
		return
	}
	id := websocketUsageRequestID(o.base.requestID, info.AuthID, info.Model, info.StartedAt)
	d := newRequestDiagnosticsCollector(o.base.manifest, id, info.StartedAt)
	d.executions[identity] = info
	d.captureLocked("client", 0, "websocket", o.clientHeader, o.clientBody, o.clientBytes)
	o.byAttempt[identity] = d
	o.byRequestID[id] = d
}

func (o *requestDiagnosticsObserver) UpstreamRequest(ctx context.Context, info cliproxyexecutor.DiagnosticRequest) {
	if d := o.collector(ctx); d != nil {
		d.upstreamRequest(ctx, info)
	}
}

func (o *requestDiagnosticsObserver) UpstreamResponse(ctx context.Context, status int, headers http.Header) {
	if d := o.collector(ctx); d != nil {
		d.response(ctx, status, headers)
	}
}

func (o *requestDiagnosticsObserver) UpstreamChunk(ctx context.Context) {
	if d := o.collector(ctx); d != nil {
		d.chunk(ctx)
	}
}

func (o *requestDiagnosticsObserver) UpstreamError(ctx context.Context, stage string, err error) {
	if d := o.collector(ctx); d != nil {
		d.upstreamError(ctx, stage, err)
	}
}

func (o *requestDiagnosticsObserver) recordResult(ctx context.Context, result coreauth.Result) {
	if d := o.collector(ctx); d != nil {
		d.result(ctx, result)
	}
}

func (o *requestDiagnosticsObserver) recordWebsocketUsage(ctx context.Context, record coreusage.Record, payload usagePayload) {
	o.mu.Lock()
	id := websocketUsageRequestID(o.base.requestID, record.AuthID, record.Model, record.RequestedAt)
	d := o.byRequestID[id]
	delete(o.byRequestID, id)
	for identity, value := range o.byAttempt {
		if value == d {
			delete(o.byAttempt, identity)
		}
	}
	o.mu.Unlock()
	if d == nil {
		return
	}
	// Preserve the upstream outcome even if the usage plugin runs after a
	// successful stream released its attempt context.
	d.result(context.Background(), coreauth.Result{Success: payload.Success})
	if !payload.Success {
		err := relayStatusError{status: payload.Status, message: payload.ErrorMessage}
		d.upstreamError(ctx, "upstream_error", err)
	}
	status := payload.Status
	if status == 0 && payload.Success {
		status = http.StatusOK
	}
	o.queue.submit(d.finish(ctx, status))
}

func (o *requestDiagnosticsObserver) finish(ctx context.Context, status int) {
	if o == nil {
		return
	}
	if !o.websocket {
		o.queue.submit(o.base.finish(ctx, status))
		return
	}
	o.mu.Lock()
	pending := o.byRequestID
	o.byRequestID = make(map[string]*requestDiagnosticsCollector)
	o.byAttempt = make(map[any]*requestDiagnosticsCollector)
	o.clientBody, o.clientHeader = nil, nil
	o.mu.Unlock()
	for _, d := range pending {
		d.mu.Lock()
		for _, attempt := range d.attempts {
			if !attempt.finished {
				attempt.finished = true
				attempt.Success = false
				attempt.LatencyMS = time.Since(attempt.started).Milliseconds()
				attempt.ErrorCategory = "request_failed"
				attempt.FailurePhase = "stream_incomplete"
				d.failurePhase = attempt.FailurePhase
			}
		}
		d.mu.Unlock()
		o.queue.submit(d.finish(ctx, status))
	}
}

// observeDirectGatewayRequest covers the fixed-provider and Ollama bridge paths
// that intentionally bypass the shared provider executors.
func observeDirectGatewayRequest(cctx context.Context, accountID, model string, headers http.Header, body []byte) {
	if observer := cliproxyexecutor.DiagnosticsObserver(cctx); observer != nil {
		observer.ExecutionStarted(cctx, cliproxyexecutor.DiagnosticExecution{AuthID: accountID, Model: model, StartedAt: time.Now()})
		observer.UpstreamRequest(cctx, cliproxyexecutor.DiagnosticRequest{AuthID: accountID, Transport: "http", Headers: headers, Body: body})
	}
}

type diagnosticResponseBody struct {
	io.ReadCloser
	ctx context.Context
}

// Keep the body observation at Read, before JSON parsing or stream conversion.
// A successful HTTP handshake and downstream keepalives are not observations.
func (b *diagnosticResponseBody) Read(p []byte) (int, error) {
	n, err := b.ReadCloser.Read(p)
	if observer := cliproxyexecutor.DiagnosticsObserver(b.ctx); observer != nil {
		if n > 0 {
			observer.UpstreamChunk(b.ctx)
		}
		if err != nil && err != io.EOF {
			observer.UpstreamError(b.ctx, "", err)
		}
	}
	return n, err
}

func observeDirectGatewayResponse(ctx context.Context, resp *http.Response) {
	if observer := cliproxyexecutor.DiagnosticsObserver(ctx); observer != nil && resp != nil {
		observer.UpstreamResponse(ctx, resp.StatusCode, resp.Header)
		if resp.Body != nil {
			resp.Body = &diagnosticResponseBody{ReadCloser: resp.Body, ctx: ctx}
		}
	}
}
