package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	runtimeexecutor "github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func outputSpeedFixture() (*requestDiagnosticsObserver, context.Context, coreusage.Record) {
	started := time.Now().Add(-20 * time.Second)
	o := newRequestDiagnosticsObserver(&manifest{}, "speed", started, false, nil)
	ctx := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	ctx = cliproxyexecutor.WithRequestDiagnosticsObserver(ctx, o)
	o.UpstreamRequest(ctx, cliproxyexecutor.DiagnosticRequest{Transport: "http"})
	o.UpstreamResponse(ctx, 200, http.Header{"Content-Type": {"text/event-stream"}})
	o.UpstreamChunk(ctx)
	o.base.attempts[0].firstFragmentAt = started.Add(5 * time.Second)
	return o, ctx, coreusage.Record{RequestedAt: started, Latency: 15 * time.Second,
		Stream: true, Detail: coreusage.Detail{OutputTokens: 1000, CachedTokens: 90000, ReasoningTokens: 200}}
}

func TestOutputSpeedCompletedStreamSurvivesClientCleanup(t *testing.T) {
	for _, tc := range []struct {
		name    string
		payload string
		want    bool
	}{
		{"completed", `{"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":100,"output_tokens":1000}}}`, true},
		{"done", `{"type":"response.done","response":{"usage":{"input_tokens":100,"output_tokens":1000}}}`, true},
		{"incomplete", `{"type":"response.incomplete","response":{"usage":{"input_tokens":100,"output_tokens":1000}}}`, false},
		{"cancelled status", `{"type":"response.completed","response":{"status":"cancelled","usage":{"input_tokens":100,"output_tokens":1000}}}`, false},
		{"terminal error", `{"type":"response.completed","response":{"error":{"code":"oops"},"usage":{"input_tokens":100,"output_tokens":1000}}}`, false},
		{"string error", `{"type":"response.completed","response":{"error":"oops","usage":{"input_tokens":100,"output_tokens":1000}}}`, false},
		{"array error", `{"type":"response.completed","response":{"error":[],"usage":{"input_tokens":100,"output_tokens":1000}}}`, false},
		{"fractional output", `{"type":"response.completed","response":{"usage":{"input_tokens":100,"output_tokens":1000.5}}}`, false},
		{"fractional input", `{"type":"response.completed","response":{"usage":{"input_tokens":100.5,"output_tokens":1000}}}`, false},
		{"negative output", `{"type":"response.completed","response":{"usage":{"input_tokens":100,"output_tokens":-1}}}`, false},
		{"partial usage", `{"type":"response.completed","response":{"usage":{"output_tokens":1000}}}`, false},
		{"empty usage", `{"type":"response.completed","response":{"usage":{}}}`, false},
		{"metadata", `{"type":"response.created","response":{"usage":{"input_tokens":100,"output_tokens":1000}}}`, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			o, ctx, record := outputSpeedFixture()
			ctx, cancel := context.WithCancel(internallogging.WithRequestID(ctx, "speed"))
			helps.ObserveCodexCompletionUsage(ctx, []byte(tc.payload))
			cancel()
			// HTTP diagnostics may close before asynchronous usage delivery.
			pending := o.base.finish(ctx, 200)
			if tc.want && pending.payload.FailurePhase != "" {
				t.Fatalf("completed output misclassified after cleanup: %s", pending.payload.FailurePhase)
			}
			tracker := newRequestUsageTracker()
			(&usagePlugin{tracker: tracker}).HandleUsage(ctx, record)
			got, _ := tracker.finalize("speed", usageFinalizeInput{status: 200, suppressOutputSpeed: true})
			if (got.OutputGenerationMS != nil) != tc.want {
				t.Fatalf("speed present=%v, want %v", got.OutputGenerationMS != nil, tc.want)
			}
		})
	}
}

func TestOutputSpeedCompletionCannotCertifyRetry(t *testing.T) {
	o, ctx, record := outputSpeedFixture()
	helps.ObserveCodexCompletionUsage(ctx, []byte(`{"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1000}}}`))
	retry := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	retry = cliproxyexecutor.WithRequestDiagnosticsObserver(retry, o)
	o.UpstreamRequest(retry, cliproxyexecutor.DiagnosticRequest{Transport: "sse"})
	if o.completedOutput(retry) {
		t.Fatal("retry inherited previous completion")
	}
	// EnsurePublished emits an empty record for a terminal event without usage.
	// It must not borrow either the previous attempt's completion or token count.
	emptyRecord := record
	emptyRecord.Detail = coreusage.Detail{}
	retry = internallogging.WithRequestID(retry, "empty-retry")
	emptyTracker := newRequestUsageTracker()
	(&usagePlugin{tracker: emptyTracker}).HandleUsage(retry, emptyRecord)
	empty, _ := emptyTracker.finalize("empty-retry", usageFinalizeInput{status: 200, suppressOutputSpeed: true})
	if empty.OutputGenerationMS != nil || empty.outputComplete {
		t.Fatal("empty usage retry inherited previous output timing")
	}
	if got := o.outputGenerationMS(ctx, record); got == nil || *got != 10000 {
		t.Fatal("retry invalidated completed timing")
	}
	tracker := newRequestUsageTracker()
	ms := int64(10000)
	tracker.record(usagePayload{RequestID: "failed", Success: true, OutputGenerationMS: &ms, outputComplete: true})
	got, _ := tracker.finalize("failed", usageFinalizeInput{status: 502, suppressOutputSpeed: true})
	if got.OutputGenerationMS != nil {
		t.Fatal("HTTP failure retained completion speed")
	}
}

func TestOutputSpeedSSEExecutionDoesNotRequireResponseContentType(t *testing.T) {
	started := time.Now().Add(-20 * time.Second)
	o := newRequestDiagnosticsObserver(&manifest{}, "speed", started, false, nil)
	ctx := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	ctx = cliproxyexecutor.WithRequestDiagnosticsObserver(ctx, o)
	helps.RecordAPIRequest(ctx, nil, helps.UpstreamRequestLog{Transport: "sse"})
	o.UpstreamResponse(ctx, 200, http.Header{"Content-Type": {"application/json"}})
	o.UpstreamChunk(ctx)
	o.base.attempts[0].firstFragmentAt = started.Add(5 * time.Second)
	record := coreusage.Record{Stream: true, RequestedAt: started, Latency: 15 * time.Second, Detail: coreusage.Detail{OutputTokens: 1000}}
	if got := o.outputGenerationMS(ctx, record); got == nil || *got != 10000 {
		t.Fatal("actual SSE execution lost timing for incorrect response content type")
	}
	if o.base.attempts[0].Transport != "sse" {
		t.Fatal("SSE diagnostic transport was not retained")
	}
}

func TestOutputSpeedCodexExecutorObservesSSECompletionWithIncorrectHeader(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte("data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n"))
		_, _ = w.Write([]byte("data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":10,\"output_tokens\":1000}}}\n\n"))
	}))
	defer server.Close()
	o := newRequestDiagnosticsObserver(&manifest{}, "speed", time.Now(), false, nil)
	ctx := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	ctx = cliproxyexecutor.WithRequestDiagnosticsObserver(ctx, o)
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	exec := runtimeexecutor.NewCodexExecutor(&config.Config{})
	stream, err := exec.ExecuteStream(ctx, &coreauth.Auth{Attributes: map[string]string{"base_url": server.URL, "api_key": "test"}},
		cliproxyexecutor.Request{Model: "gpt-5.4", Payload: []byte(`{"model":"gpt-5.4","input":"hello","stream":true}`)},
		cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: true})
	if err != nil {
		t.Fatal(err)
	}
	for chunk := range stream.Chunks {
		if chunk.Err != nil {
			t.Fatal(chunk.Err)
		}
	}
	cancel()
	if !o.completedOutput(ctx) {
		t.Fatal("executor did not certify completed usage before client cleanup")
	}
	o.base.mu.Lock()
	defer o.base.mu.Unlock()
	if len(o.base.attempts) != 1 || o.base.attempts[0].Transport != "sse" {
		t.Fatal("executor relied on Content-Type instead of its actual SSE path")
	}
}

func TestOutputSpeedWindowExcludesWaitingAndLateUsageDispatch(t *testing.T) {
	o, ctx, record := outputSpeedFixture()
	// A callback after request closure still uses the captured executor end.
	o.base.finish(context.Background(), 200)
	for i := 0; i < 2; i++ {
		got := o.outputGenerationMS(ctx, record)
		if got == nil || *got != 10000 {
			t.Fatalf("window = %v, want 10000", got)
		}
	}
	tracker := newRequestUsageTracker()
	(&usagePlugin{tracker: tracker}).HandleUsage(ctx, record)
	got, _ := tracker.finalize("", usageFinalizeInput{})
	if got.OutputGenerationMS != nil {
		t.Fatal("missing request identity gained a measurement")
	}
}

func TestOutputSpeedEligibilityAndTimeBoundaries(t *testing.T) {
	for _, tc := range []struct {
		name      string
		tokens    int64
		ms        int64
		transport string
		failed    bool
		want      bool
	}{
		{"threshold", 100, 100, "sse", false, true},
		{"short output", 99, 1000, "sse", false, false},
		{"missing usage", 0, 1000, "sse", false, false},
		{"buffered burst", 100, 99, "sse", false, false},
		{"zero window", 100, 0, "sse", false, false},
		{"reversed window", 100, -1, "sse", false, false},
		{"nonstream", 1000, 1000, "http", false, false},
		{"websocket", 1000, 1000, "websocket", false, true},
		{"failed partial output", 1000, 1000, "sse", true, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			o, ctx, record := outputSpeedFixture()
			record.Detail.OutputTokens = tc.tokens
			record.Failed = tc.failed
			record.Latency = 5*time.Second + time.Duration(tc.ms)*time.Millisecond
			o.base.attempts[0].Transport = tc.transport
			if got := o.outputGenerationMS(ctx, record); (got != nil) != tc.want {
				t.Fatalf("eligible=%v, want %v", got != nil, tc.want)
			}
		})
	}
	o, ctx, record := outputSpeedFixture()
	o.base.attempts[0].firstFragmentAt = time.Time{}
	if o.outputGenerationMS(ctx, record) != nil {
		t.Fatal("missing first fragment measured")
	}
	o, ctx, record = outputSpeedFixture()
	record.RequestedAt = time.Time{}
	if o.outputGenerationMS(ctx, record) != nil {
		t.Fatal("missing execution start measured")
	}
	if o.outputGenerationMS(context.Background(), record) != nil {
		t.Fatal("missing attempt identity fell back to another attempt")
	}
	record.Stream = false
	if o.outputGenerationMS(ctx, record) != nil {
		t.Fatal("buffered nonstream response measured as streaming")
	}
}

func TestOutputSpeedRetriesConcurrentRequestsAndCancellationStayIsolated(t *testing.T) {
	o, ctx, record := outputSpeedFixture()
	other := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	o.UpstreamRequest(other, cliproxyexecutor.DiagnosticRequest{Transport: "websocket"})
	o.UpstreamChunk(other)
	o.base.attempts[1].firstFragmentAt = record.RequestedAt.Add(14 * time.Second)
	o.base.attempts[1].FailurePhase = "client_canceled"
	if o.outputGenerationMS(other, record) != nil {
		t.Fatal("cancelled attempt measured")
	}
	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if got := o.outputGenerationMS(ctx, record); got == nil || *got != 10000 {
				t.Error("other attempt changed timing")
			}
		}()
	}
	wg.Wait()
	another, anotherCtx, anotherRecord := outputSpeedFixture()
	anotherRecord.Latency = 25 * time.Second
	if got := another.outputGenerationMS(anotherCtx, anotherRecord); got == nil || *got != 20000 {
		t.Fatal("concurrent request shared timing")
	}
	for _, input := range []usageFinalizeInput{{status: 499}, {status: 200, suppressOutputSpeed: true}} {
		tracker := newRequestUsageTracker()
		ms := int64(10000)
		tracker.record(usagePayload{RequestID: "cancel", Success: true, OutputGenerationMS: &ms})
		got, _ := tracker.finalize("cancel", input)
		if got.OutputGenerationMS != nil {
			t.Fatal("HTTP failure or cancellation retained speed")
		}
	}
}

func TestOutputSpeedUsagePluginAndWebsocketUseObservedAttempt(t *testing.T) {
	o, ctx, record := outputSpeedFixture()
	ctx = internallogging.WithRequestID(ctx, "speed")
	tracker := newRequestUsageTracker()
	plugin := &usagePlugin{tracker: tracker}
	plugin.HandleUsage(ctx, record)
	got, _ := tracker.finalize("speed", usageFinalizeInput{status: 200, latencyMS: 19000})
	if got.OutputGenerationMS == nil || *got.OutputGenerationMS != 10000 || got.LatencyMS != 19000 || got.Usage.OutputTokens != 1000 {
		t.Fatalf("usage snapshot lost or recomputed: %+v", got)
	}
	// WS has a collector per execution and emits usage once, independent of HTTP finalization.
	ws := newRequestDiagnosticsObserver(&manifest{}, "socket", record.RequestedAt, true, nil)
	wsCtx := cliproxyexecutor.WithUpstreamAttemptTracker(context.Background())
	ws.ExecutionStarted(wsCtx, cliproxyexecutor.DiagnosticExecution{StartedAt: record.RequestedAt})
	ws.UpstreamRequest(wsCtx, cliproxyexecutor.DiagnosticRequest{Transport: "websocket"})
	ws.UpstreamChunk(wsCtx)
	ws.collector(wsCtx).attempts[0].firstFragmentAt = o.base.attempts[0].firstFragmentAt
	if got := ws.outputGenerationMS(wsCtx, record); got == nil || *got != 10000 {
		t.Fatal("WS execution window missing")
	}
}
