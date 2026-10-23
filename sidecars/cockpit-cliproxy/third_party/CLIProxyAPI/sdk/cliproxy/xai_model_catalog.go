package cliproxy

import (
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

// Build-only models must not be advertised to API credentials by the default
// catalog. Explicit per-key Models settings remain authoritative at the caller.
func defaultXAIModelsForAuth(auth *coreauth.Auth) []*ModelInfo {
	models := registry.GetXAIModels()
	usingAPI := false
	if auth != nil && auth.Metadata != nil {
		switch value := auth.Metadata["using_api"].(type) {
		case bool:
			usingAPI = value
		case string:
			usingAPI = strings.EqualFold(strings.TrimSpace(value), "true")
		}
	}
	if auth != nil && auth.AuthKind() == coreauth.AuthKindOAuth && !usingAPI {
		return models
	}
	filtered := make([]*ModelInfo, 0, len(models))
	for _, model := range models {
		if model != nil && !strings.EqualFold(strings.TrimSpace(model.ID), "grok-4.7-build-fast") {
			filtered = append(filtered, model)
		}
	}
	return filtered
}
