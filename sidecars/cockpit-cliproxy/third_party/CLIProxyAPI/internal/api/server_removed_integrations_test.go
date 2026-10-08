package api

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	sdkaccess "github.com/router-for-me/CLIProxyAPI/v7/sdk/access"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func TestLegacyCompatibilityDoesNotRegisterRemovedIntegrations(t *testing.T) {
	cfg := &config.Config{
		AuthDir: t.TempDir(),
		AmpCode: config.AmpCode{UpstreamURL: "https://legacy.example", UpstreamAPIKey: "legacy-secret"},
	}
	server := NewServer(cfg, auth.NewManager(nil, nil, nil), sdkaccess.NewManager(), filepath.Join(t.TempDir(), "config.yaml"))
	for _, route := range server.engine.Routes() {
		if strings.HasPrefix(route.Path, "/v0/management/ampcode") || strings.HasPrefix(route.Path, "/api/provider") || route.Path == "/threads" {
			t.Fatalf("removed Amp integration route registered: %s", route.Path)
		}
	}
	for _, provider := range []sdktranslator.Format{sdktranslator.FormatClaude, sdktranslator.FormatCodex, sdktranslator.FormatOpenAI} {
		if sdktranslator.HasRequestTransformer("gemini-cli", provider) || sdktranslator.HasRequestTransformer(provider, "gemini-cli") {
			t.Fatalf("removed Gemini CLI translation registered for %s", provider)
		}
	}
}
