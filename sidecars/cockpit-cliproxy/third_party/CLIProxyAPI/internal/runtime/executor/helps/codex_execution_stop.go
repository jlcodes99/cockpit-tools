package helps

import (
	"errors"
	"net/http"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/clienterror"
)

// codexExecutionStop preserves the actual upstream status, quota scope and
// reset hints while forbidding replay after execution evidence. It is not
// request-scoped: a real quota rejection must still cool the account.
type codexExecutionStop struct{ error }

func (e codexExecutionStop) Unwrap() error     { return e.error }
func (codexExecutionStop) IsRequestStop() bool { return true }
func (e codexExecutionStop) StatusCode() int   { return clienterror.HTTPStatusFromError(e.error) }
func (e codexExecutionStop) RetryAfter() *time.Duration {
	var value interface{ RetryAfter() *time.Duration }
	if errors.As(e.error, &value) {
		return value.RetryAfter()
	}
	return nil
}
func (e codexExecutionStop) IsCredentialScoped() bool {
	var value interface{ IsCredentialScoped() bool }
	return errors.As(e.error, &value) && value.IsCredentialScoped()
}
func (e codexExecutionStop) Headers() http.Header {
	var value interface{ Headers() http.Header }
	if errors.As(e.error, &value) {
		return value.Headers()
	}
	return nil
}

func CodexStopAfterExecution(err error, replaySafe bool) error {
	if err == nil || replaySafe {
		return err
	}
	return codexExecutionStop{error: err}
}
