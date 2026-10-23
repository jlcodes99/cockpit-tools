package main

import (
	"context"

	"github.com/gin-gonic/gin"
)

// A committed HTTP 200 only describes the response headers. A disconnect
// before completion must still be recorded as cancellation, while cleanup
// after an observed terminal response must not turn a success into a failure.
func requestCompletionStatus(c *gin.Context, requestID string, tracker *requestUsageTracker) (int, string) {
	status := c.Writer.Status()
	message := c.Errors.String()
	if _, websocket := c.Request.Context().Value(websocketUsageContextKey).(*websocketUsageSink); websocket && status < 400 {
		// WebSocket executions own their cancellation records. Closing the
		// transport must not invent an extra failed inference request.
		return status, message
	}
	if !requestClientGone(c, nil) {
		return status, message
	}
	if observer := requestDiagnosticsFromContext(c.Request.Context()); observer != nil && !observer.websocket {
		observer.base.mu.Lock()
		attempts := observer.base.attempts
		complete := len(attempts) > 0 && attempts[len(attempts)-1].outputComplete
		observer.base.mu.Unlock()
		if complete {
			return status, message
		}
	}
	if tracker != nil {
		tracker.mu.Lock()
		records := tracker.records[requestID]
		complete := len(records) > 0 && records[len(records)-1].outputComplete
		tracker.mu.Unlock()
		if complete {
			return status, message
		}
	}
	return clientClosedRequestStatus, context.Canceled.Error()
}
