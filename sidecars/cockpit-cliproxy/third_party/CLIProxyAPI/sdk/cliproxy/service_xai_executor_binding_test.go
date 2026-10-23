package cliproxy

import (
	"net/http"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestEnsureExecutorsForAuth_XAIBindsIndependentExecutor(t *testing.T) {
	service := &Service{
		cfg:         &config.Config{},
		coreManager: coreauth.NewManager(nil, nil, nil),
	}
	auth := &coreauth.Auth{
		ID:       "xai-auth-1",
		Provider: "xai",
		Status:   coreauth.StatusActive,
		Attributes: map[string]string{
			"auth_kind": "oauth",
		},
		Metadata: map[string]any{"access_token": "xai-oauth-token"},
	}

	service.ensureExecutorsForAuth(auth)
	resolved, ok := service.coreManager.Executor("xai")
	if !ok || resolved == nil {
		t.Fatal("expected xai executor after bind")
	}
	xaiExecutor, isXAI := resolved.(*executor.XAIAutoExecutor)
	if !isXAI {
		t.Fatalf("executor type = %T, want *executor.XAIAutoExecutor", resolved)
	}
	if resolved.Identifier() != "xai" || !xaiExecutor.UsesConfig(service.cfg) {
		t.Fatal("xai executor must use its own provider identity and service config")
	}
	if _, isCodex := resolved.(*executor.CodexAutoExecutor); isCodex {
		t.Fatal("xai must not bind the codex auto executor")
	}
	if _, hasCodex := service.coreManager.Executor("codex"); hasCodex {
		t.Fatal("xai binding must not register a codex executor")
	}
	req, err := http.NewRequest(http.MethodPost, "https://api.x.ai/v1/responses", nil)
	if err != nil {
		t.Fatal(err)
	}
	if err := service.coreManager.PrepareHttpRequest(t.Context(), auth, req); err != nil {
		t.Fatal(err)
	}
	if got := req.Header.Get("Authorization"); got != "Bearer xai-oauth-token" {
		t.Fatalf("xai OAuth request authorization = %q", got)
	}
}
