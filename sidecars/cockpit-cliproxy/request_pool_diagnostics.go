package main

import (
	"context"
	"encoding/json"
	"strings"
	"sync"
)

type requestPoolDiagnosticsKey struct{}

// Priority tiers and credential retries deliberately narrow the candidate set.
// Their failures describe an attempt, not the final health of the request pool.
// Keep the latest detail on the HTTP request until its final outcome is known.
type requestPoolDiagnostics struct {
	mu        sync.Mutex
	requestID string
	pending   *requestDiagnosticPayload
	finished  bool
	recovered bool
}

// The pool is selected as "mixed", while an actual credential result names
// its backend (for example "codex"). Report successful request-level recovery
// under the pool route too, without attributing it to any individual account.
// Existing hosts can then clear an older mixed-pool failure and broadcast only
// when health actually changes, rather than refreshing the UI on every request.
func emitRequestPoolRecovery(ctx context.Context, emitter *eventEmitter, payload requestDiagnosticPayload) {
	state, _ := ctx.Value(requestPoolDiagnosticsKey{}).(*requestPoolDiagnostics)
	if state == nil || emitter == nil || state.requestID != payload.RequestID {
		return
	}
	state.mu.Lock()
	defer state.mu.Unlock()
	if state.recovered {
		return
	}
	state.recovered = true
	success := true
	payload.Success = &success
	emitter.emit(payload)
}

func withRequestPoolDiagnostics(ctx context.Context, requestID string) context.Context {
	return context.WithValue(ctx, requestPoolDiagnosticsKey{}, &requestPoolDiagnostics{requestID: requestID})
}

func deferRequestPoolDiagnostic(ctx context.Context, emitter *eventEmitter, payload requestDiagnosticPayload) bool {
	state, _ := ctx.Value(requestPoolDiagnosticsKey{}).(*requestPoolDiagnostics)
	if state == nil || state.requestID != payload.RequestID {
		return false
	}
	state.mu.Lock()
	defer state.mu.Unlock()
	if !state.finished {
		state.pending = &payload
	}
	// Retain attempt-level evidence in logs without publishing a pool failure.
	attempt := payload
	attempt.Type = "auth_pool_attempt"
	emitter.emit(attempt)
	return true
}

func finishRequestPoolDiagnostics(ctx context.Context, emitter *eventEmitter, status int, message string) {
	state, _ := ctx.Value(requestPoolDiagnosticsKey{}).(*requestPoolDiagnostics)
	if state == nil {
		return
	}
	state.mu.Lock()
	defer state.mu.Unlock()
	if state.finished {
		return
	}
	state.finished = true
	pending := state.pending
	state.pending = nil
	if pending == nil || emitter == nil || !finalRequestPoolFailure(status, message, pending.ErrorCode) {
		return
	}
	emitter.emit(*pending)
}

func finalRequestPoolFailure(status int, message, pendingCode string) bool {
	if status == clientClosedRequestStatus || strings.TrimSpace(message) == "" {
		return false
	}
	message = strings.TrimSpace(message)
	var envelope struct {
		Error struct {
			Code string `json:"code"`
		} `json:"error"`
		Response struct {
			Error struct {
				Code string `json:"code"`
			} `json:"error"`
		} `json:"response"`
	}
	code, _, _ := strings.Cut(message, ":")
	if json.Unmarshal([]byte(message), &envelope) == nil {
		code = envelope.Error.Code
		if code == "" {
			code = envelope.Response.Error.Code
		}
	}
	switch code {
	case "auth_not_found", "auth_unavailable":
		return pendingCode == "auth_not_found" || pendingCode == "auth_unavailable"
	case "account_paused", "quota_reserved":
		// Preserve upstream admission policies without replacing their final
		// cause with an earlier scope mismatch from another routing tier.
		return code == pendingCode
	}
	return false
}
