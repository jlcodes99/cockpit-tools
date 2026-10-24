package helps

import (
	"bytes"
	"context"
	"io"
	"net/http"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// Continuity can rebuild a request from canonical client history, replacing
// transport-specific tool declarations. Reapply the configured API-key model's
// collaboration projection to that final wire body, then derive response policy
// from the bytes which will actually be sent. OAuth history remains authoritative.
func BindCodexMultiAgentV2HTTPRequest(ctx context.Context, auth *cliproxyauth.Auth, req *http.Request, body []byte, headers http.Header, cfg *config.Config, isCompat bool) (*cliproxyexecutor.ContinuityAttempt, []byte, bool, error) {
	attempt, next, err := BindContinuityHTTPRequest(ctx, auth, req, body)
	if err != nil || attempt == nil || !isCompat || IsCodexOAuthAuth(auth) {
		return attempt, next, false, err
	}
	projected, optimized := OptimizeCodexMultiAgentV2RequestForAuth(ctx, headers, next, cfg, auth, true)
	if !bytes.Equal(projected, next) {
		req.Body = io.NopCloser(bytes.NewReader(projected))
		req.ContentLength = int64(len(projected))
		req.GetBody = func() (io.ReadCloser, error) { return io.NopCloser(bytes.NewReader(projected)), nil }
	}
	return attempt, projected, optimized, nil
}
