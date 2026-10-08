package executor

import (
	"context"
	"net/http"
	"time"
)

// DiagnosticRequest is observed after translation, immediately before transport.
// Observers must bound captured bodies and must never block on external I/O.
type DiagnosticRequest struct {
	Provider, AuthID, Transport string
	Headers                     http.Header
	Body                        []byte
}

type DiagnosticExecution struct {
	Provider, AuthID, Model string
	StartedAt               time.Time
}

// RequestDiagnosticsObserver observes actual upstream activity independently of
// request-file logging. Payload persistence and redaction belong to the caller.
type RequestDiagnosticsObserver interface {
	ClientPayload(context.Context, []byte, http.Header, string)
	ExecutionStarted(context.Context, DiagnosticExecution)
	UpstreamRequest(context.Context, DiagnosticRequest)
	UpstreamResponse(context.Context, int, http.Header)
	UpstreamChunk(context.Context)
	UpstreamError(context.Context, string, error)
}

type requestDiagnosticsContextKey struct{}
type requestDiagnosticsContextValue struct{ observer RequestDiagnosticsObserver }

func WithRequestDiagnosticsObserver(ctx context.Context, observer RequestDiagnosticsObserver) context.Context {
	if ctx == nil {
		ctx = context.Background()
	}
	return context.WithValue(ctx, requestDiagnosticsContextKey{}, requestDiagnosticsContextValue{observer: observer})
}

func DiagnosticsObserver(ctx context.Context) RequestDiagnosticsObserver {
	if ctx == nil {
		return nil
	}
	value, _ := ctx.Value(requestDiagnosticsContextKey{}).(requestDiagnosticsContextValue)
	return value.observer
}

// UpstreamAttemptIdentity is an opaque identity for a single executor call. It
// prevents late callbacks from a cancelled attempt from changing its successor.
func UpstreamAttemptIdentity(ctx context.Context) any {
	if ctx == nil {
		return nil
	}
	return ctx.Value(upstreamAttemptTrackerContextKey{})
}

func ObserveUpstreamChunk(ctx context.Context) {
	if observer := DiagnosticsObserver(ctx); observer != nil {
		observer.UpstreamChunk(ctx)
	}
}
