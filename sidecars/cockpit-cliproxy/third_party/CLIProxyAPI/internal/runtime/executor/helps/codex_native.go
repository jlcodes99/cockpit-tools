package helps

import (
	"strings"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

// IsNativeCodexRequest reports whether the request already uses the native
// Codex/Responses dialect and can bypass compatibility normalization.
func IsNativeCodexRequest(body []byte, opts cliproxyexecutor.Options) bool {
	for _, format := range []sdktranslator.Format{opts.SourceFormat, cliproxyexecutor.ResponseFormatOrSource(opts)} {
		name := strings.TrimSpace(format.String())
		if !strings.EqualFold(name, sdktranslator.FormatCodex.String()) && !strings.EqualFold(name, sdktranslator.FormatOpenAIResponse.String()) {
			return false
		}
	}
	// The executor's Responses-Lite detector is package-private. Native
	// requests are already in the Codex dialect; compatibility filtering is
	// therefore unnecessary here and is handled by the executor's dedicated
	// Responses-Lite stage when enabled.
	return len(body) > 0
}
