package helps

import (
	"context"
	"net/http"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// DoGuardedUpstreamHTTP runs after credential/protocol preparation and before
// each HTTP attempt, including a same-account recovery retry.
func DoGuardedUpstreamHTTP(ctx context.Context, auth *cliproxyauth.Auth, client *http.Client, request *http.Request) (*http.Response, error) {
	authID := ""
	if auth != nil {
		authID = auth.ID
	}
	if err := cliproxyexecutor.CheckUpstreamSend(ctx, authID); err != nil {
		return nil, err
	}
	return client.Do(request)
}
