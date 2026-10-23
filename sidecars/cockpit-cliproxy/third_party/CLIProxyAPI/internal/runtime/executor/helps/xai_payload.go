package helps

import (
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// NewXAIPayloadFinalizer defers user payload rules until built-in adaptation is
// complete. Retry paths pass the prepared payload again, not a finalized copy.
func NewXAIPayloadFinalizer(cfg *config.Config, model, protocol, from string, original []byte, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) func([]byte) []byte {
	requestedModel := PayloadRequestedModel(opts, req.Model)
	requestPath := PayloadRequestPath(opts)
	return func(body []byte) []byte {
		return ApplyPayloadConfigWithRequest(cfg, model, protocol, from, "", body, original, requestedModel, requestPath, opts.Headers)
	}
}
