package executor

import (
	"context"
	"errors"
)

// UpstreamSendGuard rechecks host-owned admission immediately before each
// generation send. Implementations must use fast in-memory state and must not
// wait for disk, network, or a worker while a transport write lock is held.
type UpstreamSendGuard func(context.Context, string) error

type upstreamSendGuardKey struct{}

// WithUpstreamSendGuard attaches a per-request guard. Credential refreshes and
// transport retries inherit it without relying on executor registration order.
func WithUpstreamSendGuard(ctx context.Context, guard UpstreamSendGuard) context.Context {
	if ctx == nil {
		ctx = context.Background()
	}
	return context.WithValue(ctx, upstreamSendGuardKey{}, guard)
}

type upstreamSendGuardError struct{ cause error }

func (e *upstreamSendGuardError) Error() string { return e.cause.Error() }
func (e *upstreamSendGuardError) Unwrap() error { return e.cause }

// CheckUpstreamSend preserves typed host errors for account selection. A guard
// rejection is local and must never be retried as a disconnected socket write.
func CheckUpstreamSend(ctx context.Context, authID string) error {
	if ctx == nil {
		return nil
	}
	if err := ctx.Err(); err != nil {
		return &upstreamSendGuardError{cause: err}
	}
	guard, _ := ctx.Value(upstreamSendGuardKey{}).(UpstreamSendGuard)
	if guard != nil {
		if err := guard(ctx, authID); err != nil {
			return &upstreamSendGuardError{cause: err}
		}
	}
	return nil
}

func IsUpstreamSendGuardError(err error) bool {
	var rejected *upstreamSendGuardError
	return errors.As(err, &rejected)
}
