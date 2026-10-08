package main

import (
	"context"
	"errors"
	"io"
	"net/http"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

const (
	maxDiagnosticAttempts       = 32
	maxDiagnosticPayloads       = 8
	maxDiagnosticSnapshotBytes  = 16 << 10
	maxDiagnosticTotalBodyBytes = 64 << 10
	maxDiagnosticRawBodyBytes   = 256 << 10
	requestDiagnosticsQueueSize = 64
	maxDiagnosticEventBytes     = 64 << 10
)

type requestAttemptDetail struct {
	Sequence      int    `json:"sequence"`
	AccountID     string `json:"accountId"`
	AccountEmail  string `json:"accountEmail,omitempty"`
	ModelID       string `json:"modelId"`
	Transport     string `json:"transport"`
	StartedAtMS   int64  `json:"startedAtMs"`
	LatencyMS     int64  `json:"latencyMs"`
	Status        int    `json:"status,omitempty"`
	Success       bool   `json:"success"`
	ErrorCategory string `json:"errorCategory,omitempty"`
	ErrorMessage  string `json:"errorMessage,omitempty"`
	FailurePhase  string `json:"failurePhase,omitempty"`
}

type requestPayloadSnapshot struct {
	Stage           string            `json:"stage"`
	AttemptSequence int               `json:"attemptSequence,omitempty"`
	Transport       string            `json:"transport"`
	ContentType     string            `json:"contentType"`
	Headers         map[string]string `json:"headers,omitempty"`
	Body            string            `json:"body"`
	Truncated       bool              `json:"truncated"`
	OriginalBytes   int               `json:"originalBytes"`
	SHA256          string            `json:"sha256"`
}

type requestDiagnosticsPayload struct {
	Type            string                   `json:"type"`
	RequestID       string                   `json:"requestId"`
	FirstResponseMS *int64                   `json:"firstResponseMs,omitempty"`
	FailurePhase    string                   `json:"failurePhase,omitempty"`
	Attempts        []requestAttemptDetail   `json:"attempts"`
	Payloads        []requestPayloadSnapshot `json:"payloads"`
	Truncated       bool                     `json:"truncated,omitempty"`
	CapturedAtMS    int64                    `json:"capturedAtMs"`
}

type rawDiagnosticSnapshot struct {
	requestPayloadSnapshot
	body []byte
}

type diagnosticAttempt struct {
	requestAttemptDetail
	identity any
	started  time.Time
	finished bool
	first    bool
}

// Request-owned collectors retain bounded memory and never write to stdout or
// the filesystem. The worker redacts snapshots into a bounded HTTP IPC queue.
type requestDiagnosticsCollector struct {
	mu              sync.Mutex
	manifest        *manifest
	capturePayloads bool
	requestID       string
	started         time.Time
	firstResponseMS *int64
	attempts        []*diagnosticAttempt
	executions      map[any]cliproxyexecutor.DiagnosticExecution
	rawPayloads     []rawDiagnosticSnapshot
	rawBodyBytes    int
	failurePhase    string
	truncated       bool
	closed          bool
}

func newRequestDiagnosticsCollector(m *manifest, id string, started time.Time) *requestDiagnosticsCollector {
	return &requestDiagnosticsCollector{manifest: m, capturePayloads: m.payloadLoggingEnabled(), requestID: id, started: started, executions: make(map[any]cliproxyexecutor.DiagnosticExecution)}
}

func (d *requestDiagnosticsCollector) latestLocked(ctx context.Context) *diagnosticAttempt {
	identity := cliproxyexecutor.UpstreamAttemptIdentity(ctx)
	for i := len(d.attempts) - 1; i >= 0; i-- {
		if identity == nil || d.attempts[i].identity == identity {
			return d.attempts[i]
		}
	}
	return nil
}

func (d *requestDiagnosticsCollector) captureLocked(stage string, sequence int, transport string, headers http.Header, body []byte, originalBytes int) {
	if d.closed || !d.capturePayloads {
		return
	}
	if len(d.rawPayloads) >= maxDiagnosticPayloads {
		d.truncated = true
		return
	}
	snapshot := rawDiagnosticSnapshot{requestPayloadSnapshot: requestPayloadSnapshot{
		Stage: stage, AttemptSequence: sequence, Transport: transport, ContentType: headers.Get("Content-Type"),
		Headers: safeDiagnosticHeaders(headers), OriginalBytes: originalBytes,
	}}
	// Never redact an arbitrary JSON prefix: it may end inside a credential.
	// Oversized bodies are explicitly omitted instead of leaking partial JSON.
	if len(body) != originalBytes || len(body) > maxDiagnosticRawBodyBytes-d.rawBodyBytes {
		snapshot.Truncated = true
		d.truncated = true
	} else {
		snapshot.body = append([]byte(nil), body...)
		d.rawBodyBytes += len(body)
	}
	d.rawPayloads = append(d.rawPayloads, snapshot)
}

func (d *requestDiagnosticsCollector) upstreamRequest(ctx context.Context, info cliproxyexecutor.DiagnosticRequest) {
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.closed {
		return
	}
	if len(d.attempts) >= maxDiagnosticAttempts {
		d.truncated = true
		return
	}
	now := time.Now()
	identity := cliproxyexecutor.UpstreamAttemptIdentity(ctx)
	execution := d.executions[identity]
	model := strings.TrimSpace(gjson.GetBytes(info.Body, "model").String())
	if model == "" {
		model = execution.Model
	}
	accountID, email := info.AuthID, ""
	if account := accountForAuthIDInManifest(d.manifest, info.AuthID); account != nil {
		accountID, email = account.ID, account.Email
	}
	a := &diagnosticAttempt{requestAttemptDetail: requestAttemptDetail{
		Sequence: len(d.attempts) + 1, AccountID: accountID, AccountEmail: email, ModelID: model,
		Transport: info.Transport, StartedAtMS: now.UnixMilli(),
	}, identity: identity, started: now}
	d.attempts = append(d.attempts, a)
	d.captureLocked("upstream", a.Sequence, info.Transport, info.Headers, info.Body, len(info.Body))
}

func accountForAuthIDInManifest(m *manifest, id string) *accountSpec {
	if m == nil {
		return nil
	}
	if account := m.accountByAuthID[strings.ToLower(strings.TrimSpace(id))]; account != nil {
		return account
	}
	return m.accountByID[strings.TrimSpace(id)]
}

func (d *requestDiagnosticsCollector) response(ctx context.Context, status int, headers http.Header) {
	d.mu.Lock()
	defer d.mu.Unlock()
	if a := d.latestLocked(ctx); !d.closed && a != nil {
		a.Status = status
		if a.Transport == "http" && strings.Contains(strings.ToLower(headers.Get("Content-Type")), "text/event-stream") {
			a.Transport = "sse"
		}
	}
}

func (d *requestDiagnosticsCollector) chunk(ctx context.Context) {
	d.mu.Lock()
	defer d.mu.Unlock()
	a := d.latestLocked(ctx)
	if d.closed || a == nil {
		return
	}
	a.first = true
	if d.firstResponseMS == nil {
		value := time.Since(d.started).Milliseconds()
		d.firstResponseMS = &value
	}
}

func (d *requestDiagnosticsCollector) upstreamError(ctx context.Context, stage string, err error) {
	if err == nil {
		return
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	a := d.latestLocked(ctx)
	if d.closed || a == nil {
		return
	}
	a.Success = false
	a.ErrorMessage = safeDiagnosticError(err.Error())
	a.ErrorCategory = errorCategory(a.Status, a.ErrorMessage, false)
	a.LatencyMS = time.Since(a.started).Milliseconds()
	if stage == "" && a.Status == 0 && !a.first {
		stage = "dial"
	}
	a.FailurePhase = diagnosticFailurePhase(ctx, stage, a.first, err)
	d.failurePhase = a.FailurePhase
	a.finished = true
}

func diagnosticFailurePhase(ctx context.Context, stage string, first bool, err error) string {
	if errors.Is(err, context.Canceled) || ctx != nil && errors.Is(ctx.Err(), context.Canceled) {
		return "client_canceled"
	}
	if errors.Is(err, context.DeadlineExceeded) || strings.Contains(err.Error(), "upstream timed out") {
		if first {
			return "stream_read"
		}
		return "upstream_response"
	}
	switch stage {
	case "dial", "dial_retry", "send", "send_retry":
		return "upstream_connect"
	case "upstream_error":
		return "upstream_response"
	}
	if first {
		return "stream_read"
	}
	return "upstream_response"
}

func (d *requestDiagnosticsCollector) result(ctx context.Context, result coreauth.Result) {
	d.mu.Lock()
	defer d.mu.Unlock()
	a := d.latestLocked(ctx)
	if d.closed || a == nil {
		return
	}
	a.LatencyMS = time.Since(a.started).Milliseconds()
	a.finished = true
	a.Success = result.Success
	if result.Error != nil {
		a.Status = result.Error.HTTPStatus
		a.ErrorMessage = safeDiagnosticError(result.Error.Message)
		a.ErrorCategory = errorCategory(a.Status, a.ErrorMessage, false)
		if a.FailurePhase == "" {
			a.FailurePhase = "upstream_response"
		}
		d.failurePhase = a.FailurePhase
	} else if result.Success {
		a.ErrorCategory, a.ErrorMessage, a.FailurePhase = "", "", ""
		d.failurePhase = ""
	}
}

type pendingRequestDiagnostics struct {
	payload requestDiagnosticsPayload
	raw     []rawDiagnosticSnapshot
}

func (d *requestDiagnosticsCollector) finish(ctx context.Context, status int) pendingRequestDiagnostics {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.closed = true
	if len(d.attempts) == 0 && status >= 400 {
		d.failurePhase = "selection"
	}
	attempts := make([]requestAttemptDetail, 0, len(d.attempts))
	for _, a := range d.attempts {
		if !a.finished {
			a.LatencyMS = time.Since(a.started).Milliseconds()
			if ctx != nil && ctx.Err() != nil {
				a.Success = false
				a.ErrorCategory = "client_canceled"
				a.FailurePhase = diagnosticFailurePhase(ctx, "", a.first, ctx.Err())
				d.failurePhase = a.FailurePhase
			} else {
				a.Success = status >= 200 && status < 400
				if !a.Success {
					a.Status = status
					a.ErrorCategory = errorCategory(status, "", false)
					a.FailurePhase = "upstream_response"
					d.failurePhase = a.FailurePhase
				}
			}
		}
		attempts = append(attempts, a.requestAttemptDetail)
	}
	return pendingRequestDiagnostics{payload: requestDiagnosticsPayload{
		Type: "request_diagnostics", RequestID: d.requestID, FirstResponseMS: d.firstResponseMS,
		FailurePhase: d.failurePhase, Attempts: attempts, Payloads: []requestPayloadSnapshot{},
		Truncated: d.truncated, CapturedAtMS: time.Now().UnixMilli(),
	}, raw: d.rawPayloads}
}

type requestDiagnosticsQueue struct {
	pending chan pendingRequestDiagnostics
	ready   chan requestDiagnosticsPayload
	stop    chan struct{}
	done    chan struct{}
	once    sync.Once
	dropped atomic.Uint64
	emit    func(requestDiagnosticsPayload)
}

func newRequestDiagnosticsQueue(emit func(requestDiagnosticsPayload)) *requestDiagnosticsQueue {
	q := &requestDiagnosticsQueue{pending: make(chan pendingRequestDiagnostics, requestDiagnosticsQueueSize), ready: make(chan requestDiagnosticsPayload, requestDiagnosticsQueueSize), stop: make(chan struct{}), done: make(chan struct{}), emit: emit}
	go q.run()
	return q
}

func (q *requestDiagnosticsQueue) run() {
	defer close(q.done)
	for {
		select {
		case <-q.stop:
			return
		default:
		}
		select {
		case <-q.stop:
			return
		case pending := <-q.pending:
			prepareDiagnosticSnapshots(&pending)
			limitDiagnosticEvent(&pending.payload)
			if q.emit != nil {
				q.emit(pending.payload)
			} else {
				select {
				case q.ready <- pending.payload:
				default:
					q.dropped.Add(1)
				}
			}
		}
	}
}

func (q *requestDiagnosticsQueue) submit(pending pendingRequestDiagnostics) {
	if q == nil {
		return
	}
	select {
	case q.pending <- pending:
	default:
		// The host polls this cumulative counter and logs only its delta. No
		// request or diagnostic worker waits on stdout/stderr logging locks.
		q.dropped.Add(1)
	}
}

func (q *requestDiagnosticsQueue) close() {
	if q != nil {
		q.once.Do(func() { close(q.stop) })
	}
}

func (p *requestPolicy) stopRequestDiagnostics() {
	if p != nil {
		p.diagnosticsOnce.Do(func() {})
		p.diagnosticsQueue.close()
	}
}

func (p *requestPolicy) diagnosticsWorker() *requestDiagnosticsQueue {
	if p == nil || p.emitter == nil {
		return nil
	}
	p.diagnosticsOnce.Do(func() {
		p.diagnosticsQueue = newRequestDiagnosticsQueue(nil)
	})
	return p.diagnosticsQueue
}

// Body capture is attached before policy rewriting, so the client snapshot is
// the body actually received. Large/malformed/binary bodies are safely omitted.
type diagnosticClientBody struct {
	io.ReadCloser
	observer *requestDiagnosticsObserver
	ctx      context.Context
	headers  http.Header
	body     []byte
	bytes    int
	done     bool
}

func (b *diagnosticClientBody) Read(p []byte) (int, error) {
	n, err := b.ReadCloser.Read(p)
	b.bytes += n
	if b.bytes <= maxDiagnosticRawBodyBytes {
		b.body = append(b.body, p[:n]...)
	} else {
		b.body = nil
	}
	if err != nil && !b.done {
		b.done = true
		b.observer.captureClient(b.ctx, b.body, b.headers, "http", b.bytes)
	}
	return n, err
}

func (p *requestPolicy) beginRequestDiagnostics(c *gin.Context, id string, started time.Time) *requestDiagnosticsObserver {
	if !shouldEmitRequestDiagnostic(c.Request) {
		return nil
	}
	observer := newRequestDiagnosticsObserver(p.manifest, id, started, diagnosticTransport(c.Request) == "websocket", p.diagnosticsWorker())
	c.Request = c.Request.WithContext(cliproxyexecutor.WithRequestDiagnosticsObserver(c.Request.Context(), observer))
	if observer.base.capturePayloads && c.Request.Body != nil && !observer.websocket {
		c.Request.Body = &diagnosticClientBody{ReadCloser: c.Request.Body, observer: observer, ctx: c.Request.Context(), headers: c.Request.Header.Clone()}
	}
	return observer
}
