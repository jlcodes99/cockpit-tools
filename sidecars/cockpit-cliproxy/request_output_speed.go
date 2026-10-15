package main

import (
	"context"
	"time"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

// A first upstream fragment can be metadata or buffered text. This measures an
// approximate output rate, never a tokenizer estimate or a precise TTFT.
const outputSpeedMinTokens int64 = 100
const outputSpeedMinWindow = 100 * time.Millisecond

func (o *requestDiagnosticsObserver) outputGenerationMS(ctx context.Context, record coreusage.Record) *int64 {
	if o == nil || !record.Stream || record.Failed || record.Detail.OutputTokens < outputSpeedMinTokens ||
		record.RequestedAt.IsZero() || record.Latency <= 0 || cliproxyexecutor.UpstreamAttemptIdentity(ctx) == nil {
		return nil
	}
	d := o.collector(ctx)
	if d == nil {
		return nil
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	a := d.latestLocked(ctx)
	if a == nil || (a.Transport != "sse" && a.Transport != "websocket") ||
		(a.outputTerminal && !a.outputComplete) || a.firstFragmentAt.IsZero() || a.firstFragmentAt.Before(record.RequestedAt) || a.FailurePhase != "" {
		return nil
	}
	// Use the executor's captured completion, not the time an asynchronous usage
	// plugin runs. The fragment belongs to this exact upstream attempt, so retries
	// and other requests cannot contribute their waiting or output time.
	window := record.RequestedAt.Add(record.Latency).Sub(a.firstFragmentAt)
	if window < outputSpeedMinWindow {
		return nil
	}
	ms := window.Milliseconds()
	return &ms
}

// Completion is tied to the exact upstream attempt, before asynchronous usage
// dispatch. Context cancellation after this event is only connection cleanup.
func (o *requestDiagnosticsObserver) UpstreamCompletion(ctx context.Context, complete bool) {
	if cliproxyexecutor.UpstreamAttemptIdentity(ctx) == nil {
		return
	}
	if d := o.collector(ctx); d != nil {
		d.mu.Lock()
		defer d.mu.Unlock()
		if a := d.latestLocked(ctx); !d.closed && a != nil {
			a.outputTerminal, a.outputComplete = true, complete
		}
	}
}

func (o *requestDiagnosticsObserver) completedOutput(ctx context.Context) bool {
	if cliproxyexecutor.UpstreamAttemptIdentity(ctx) == nil {
		return false
	}
	if d := o.collector(ctx); d != nil {
		d.mu.Lock()
		defer d.mu.Unlock()
		if a := d.latestLocked(ctx); a != nil {
			return a.outputComplete
		}
	}
	return false
}
