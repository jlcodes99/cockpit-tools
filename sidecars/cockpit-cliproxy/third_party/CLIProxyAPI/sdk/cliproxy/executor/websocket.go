package executor

import (
	"context"
	"errors"
	"net/http"
)

// ErrNoActiveUpstreamWebsocket means response.interrupt has no live upstream socket.
var ErrNoActiveUpstreamWebsocket = errors.New("no active upstream websocket for response.interrupt")

// WithWebsocketAuthCheck supplies the live account-state check for a bound
// connection. It may reject further frames but never select another account.
type websocketAuthCheckKey struct{}

func WithWebsocketAuthCheck(ctx context.Context, check func(string) bool) context.Context {
	return context.WithValue(ctx, websocketAuthCheckKey{}, check)
}

func WebsocketAuthEnabled(ctx context.Context, authID string) bool {
	if ctx == nil {
		return true
	}
	check, _ := ctx.Value(websocketAuthCheckKey{}).(func(string) bool)
	return check == nil || check(authID)
}

// UpstreamWebsocketReplayRequiredError indicates that an incremental request
// cannot safely continue because its upstream websocket is no longer reusable.
type UpstreamWebsocketReplayRequiredError struct{}

func (*UpstreamWebsocketReplayRequiredError) Error() string {
	return `{"error":{"message":"upstream transport requires full HTTP replay","type":"server_error","code":"upstream_http_replay_required","status":426}}`
}

func (*UpstreamWebsocketReplayRequiredError) StatusCode() int { return http.StatusUpgradeRequired }

func (*UpstreamWebsocketReplayRequiredError) IsRequestScoped() bool { return true }

// NewUpstreamWebsocketReplayRequiredError creates a request-scoped replay signal.
func NewUpstreamWebsocketReplayRequiredError() error {
	return &UpstreamWebsocketReplayRequiredError{}
}

// IsUpstreamWebsocketReplayRequired reports whether err is the internal replay signal.
func IsUpstreamWebsocketReplayRequired(err error) bool {
	var replayErr *UpstreamWebsocketReplayRequiredError
	return errors.As(err, &replayErr)
}
