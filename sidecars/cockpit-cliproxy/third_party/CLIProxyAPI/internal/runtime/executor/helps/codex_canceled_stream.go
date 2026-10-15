package helps

import (
	"context"
	"errors"

	api "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

// SendCodexWebsocketChunk stops ordinary output on cancellation while retaining
// a confirmed failure in the reserved error slot. The caller reserves one slot
// beyond its initial payload buffer and has a single producer.
func SendCodexWebsocketChunk(ctx context.Context, out chan api.StreamChunk, chunk api.StreamChunk) bool {
	if ctx == nil {
		out <- chunk
		return true
	}
	if ctx.Err() == nil {
		select {
		case out <- chunk:
			return true
		case <-ctx.Done():
		}
	}
	if chunk.Err == nil || errors.Is(chunk.Err, context.Canceled) || errors.Is(chunk.Err, context.DeadlineExceeded) {
		return false
	}
	// Queued normal output is no longer deliverable. Free the one error slot
	// without waiting for a consumer, rather than dropping the confirmed failure.
	for {
		select {
		case <-out:
		default:
			out <- chunk
			return true
		}
	}
}

// Only failure frames need parsing after client cancellation. Normal queued
// completions must not turn a canceled attempt into a success or mutate replay.
func CodexWebsocketFailureCandidate(payload []byte) bool {
	switch gjson.GetBytes(payload, "type").String() {
	case "error", "response.failed", "response.incomplete":
		return true
	case "response.done":
		return gjson.GetBytes(payload, "response.error").IsObject()
	default:
		return false
	}
}
