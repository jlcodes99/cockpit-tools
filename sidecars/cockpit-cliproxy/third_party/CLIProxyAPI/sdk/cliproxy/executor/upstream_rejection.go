package executor

import "errors"

// UpstreamRequestRejected marks an HTTP rejection received before a generation
// stream was accepted. Execution-time SSE/WS errors must never use this marker.
func UpstreamRequestRejected(err error) error {
	if err == nil {
		return nil
	}
	return &upstreamRequestRejectedError{cause: err}
}

type upstreamRequestRejectedError struct{ cause error }

func (e *upstreamRequestRejectedError) Error() string { return e.cause.Error() }
func (e *upstreamRequestRejectedError) Unwrap() error { return e.cause }
func IsUpstreamRequestRejected(err error) bool {
	var rejected *upstreamRequestRejectedError
	return errors.As(err, &rejected)
}
