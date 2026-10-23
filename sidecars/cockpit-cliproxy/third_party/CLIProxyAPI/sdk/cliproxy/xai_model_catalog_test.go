package cliproxy

import (
	"context"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestXAIModelCatalogKeepsBuildFastScopedToBuild(t *testing.T) {
	for _, tt := range []struct {
		name      string
		auth      *coreauth.Auth
		wantBuild bool
	}{
		{"api key", &coreauth.Auth{Attributes: map[string]string{"api_key": "test"}}, false},
		{"unknown credential", &coreauth.Auth{}, false},
		{"build oauth", &coreauth.Auth{Metadata: map[string]any{"auth_kind": "oauth"}}, true},
		{"oauth api mode", &coreauth.Auth{Metadata: map[string]any{"auth_kind": "oauth", "using_api": true}}, false},
		{"oauth api string", &coreauth.Auth{Metadata: map[string]any{"auth_kind": "oauth", "using_api": "true"}}, false},
	} {
		t.Run(tt.name, func(t *testing.T) {
			found, foundBuild := false, false
			for _, model := range defaultXAIModelsForAuth(tt.auth) {
				found = found || model.ID == "grok-4.7"
				foundBuild = foundBuild || model.ID == "grok-4.7-build-fast"
			}
			if !found || foundBuild != tt.wantBuild {
				t.Fatalf("grok-4.7=%v build-fast=%v, want true/%v", found, foundBuild, tt.wantBuild)
			}
		})
	}
}

func TestXAIModelCatalogPreservesExplicitAPIKeyModels(t *testing.T) {
	auth := &coreauth.Auth{ID: "grok-catalog-explicit-key", Provider: "xai", Attributes: map[string]string{"api_key": "test-key"}}
	service := &Service{cfg: &config.Config{XAIKey: []config.XAIKey{{
		APIKey: "test-key", Models: []config.XAIModel{
			{Name: "custom-grok", Alias: "custom-grok"},
			{Name: "grok-4.7-build-fast", Alias: "grok-4.7-build-fast"},
		},
	}}}}
	t.Cleanup(func() { GlobalModelRegistry().UnregisterClient(auth.ID) })
	service.registerModelsForAuth(context.Background(), auth)
	models := registry.GetGlobalRegistry().GetModelsForClient(auth.ID)
	if len(models) != 2 {
		t.Fatalf("configured model count = %d, want 2", len(models))
	}
	for _, model := range models {
		if model.ID != "custom-grok" && model.ID != "grok-4.7-build-fast" {
			t.Fatalf("default model leaked into explicit list: %q", model.ID)
		}
	}
}
