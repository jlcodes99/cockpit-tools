package main

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"testing"
	"time"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

func TestExplicitUpstreamStreamErrorsKeepUsageAndDiagnosticClassification(t *testing.T) {
	for _, tc := range []struct {
		code, message, category string
		status                  int
	}{
		{"upstream_stream_error", "unexpected EOF", "upstream_stream_error", 502},
		{"upstream_stream_error", "read tcp: connection reset by peer", "upstream_stream_error", 502},
		{"upstream_stream_error", "peer reported timeout before reset", "upstream_stream_error", 502},
		{"incomplete_upstream_stream", "stream closed before response.completed", "incomplete_upstream_stream", 502},
		{"request_timeout", "read tcp: i/o timeout", "upstream_stream_timeout", 408},
	} {
		t.Run(tc.code+tc.message, func(t *testing.T) {
			body := fmt.Sprintf(`{"error":{"code":%q,"type":"server_error","message":%q}}`, tc.code, tc.message)
			tracker := newRequestUsageTracker()
			plugin := &usagePlugin{tracker: tracker}
			ctx := internallogging.WithRequestID(context.Background(), t.Name())
			plugin.HandleUsage(ctx, coreusage.Record{
				Provider: "codex", Failed: true, RequestedAt: time.Now(),
				Fail: coreusage.Failure{StatusCode: tc.status, Body: body},
			})
			payload, ok := tracker.finalize(t.Name(), usageFinalizeInput{status: tc.status, completedAtMS: time.Now().UnixMilli()})
			if !ok || payload.ErrorCategory != tc.category || payload.ErrorMessage != body {
				t.Fatalf("upstream failure classified as cancellation or cause lost: %#v", payload)
			}
			diagnostics := newRequestDiagnosticsCollector(&manifest{}, t.Name(), time.Now())
			diagnostics.upstreamRequest(ctx, cliproxyexecutor.DiagnosticRequest{Transport: "sse", AuthID: "test"})
			diagnostics.response(ctx, http.StatusOK, http.Header{"Content-Type": {"text/event-stream"}})
			diagnostics.chunk(ctx)
			diagnostics.upstreamError(ctx, "read", errors.New(body))
			if got := diagnostics.attempts[0].ErrorCategory; got != tc.category {
				t.Fatalf("diagnostic error category = %q, want %q", got, tc.category)
			}
			if got := errorCategory(tc.status, `{"type":"response.failed","response":`+body+`}`, false); got != tc.category {
				t.Fatalf("nested response failure category = %q, want %q", got, tc.category)
			}
		})
	}
	if got := errorCategory(http.StatusBadGateway, "write tcp: broken pipe", false); got != "client_canceled" {
		t.Fatalf("downstream write failure changed category: %q", got)
	}
}
