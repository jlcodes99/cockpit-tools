package main

import "context"

const requestStartedAtContextKey contextKey = "cockpitRequestStartedAt"

// Keep attempt results tied to the original request start, even when a slow
// request finishes after a newer pool failure for the same key and model.
func requestStartedAtMS(ctx context.Context) int64 {
	if ctx == nil {
		return 0
	}
	startedAt, _ := ctx.Value(requestStartedAtContextKey).(int64)
	return startedAt
}
