package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

// Cockpit 的 Grok 供应商账号以 xai OAuth auth 文件交付给 sidecar：
// provider 必须来自 auth 文件的 type 字段，并且注册为 xai 账号。
func TestReadManifestTokenAuthAcceptsXAIOAuth(t *testing.T) {
	authDir := t.TempDir()
	path := filepath.Join(authDir, "xai-codex_grok_test.json")
	payload := map[string]any{
		"type":         "xai",
		"auth_kind":    "oauth",
		"access_token": "xai-access-token",
		"token_type":   "Bearer",
		"email":        "grok@example.com",
		"base_url":     "https://cli-chat-proxy.grok.com/v1",
		"using_api":    false,
	}
	data, err := json.Marshal(payload)
	if err != nil {
		t.Fatalf("marshal auth: %v", err)
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatalf("write auth: %v", err)
	}

	auth, err := readManifestCodexTokenAuth(&accountSpec{
		ID:       "codex-grok-test",
		Email:    "grok@example.com",
		AuthID:   "xai-codex_grok_test.json",
		AuthKind: "oauth",
		Provider: "xai",
		ModelIDs: []string{"grok-4.6"},
	}, authDir, path)
	if err != nil {
		t.Fatalf("read xai auth: %v", err)
	}
	if auth.Provider != "xai" {
		t.Fatalf("provider = %q, want xai", auth.Provider)
	}
	if got := auth.Metadata["access_token"]; got != "xai-access-token" {
		t.Fatalf("access token = %v", got)
	}
	if auth.Attributes["account_id"] != "codex-grok-test" {
		t.Fatalf("account id = %q", auth.Attributes["account_id"])
	}
	if _, exists := auth.Attributes["websockets"]; exists {
		t.Fatal("xai auth must not force the codex websocket attribute")
	}
}

func TestReadManifestTokenAuthRejectsUnsupportedProvider(t *testing.T) {
	authDir := t.TempDir()
	path := filepath.Join(authDir, "other.json")
	data, err := json.Marshal(map[string]any{
		"type":         "openai",
		"access_token": "token",
	})
	if err != nil {
		t.Fatalf("marshal auth: %v", err)
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatalf("write auth: %v", err)
	}
	if _, err := readManifestCodexTokenAuth(nil, authDir, path); err == nil {
		t.Fatal("unsupported provider must be rejected")
	}
}

// 原生 provider 路由（Grok）：命名空间下的模型直接交给 xai 执行器，不走 Provider Gateway。
func TestResolveModelRoutingRouteReturnsNativeProvider(t *testing.T) {
	spec := &apiKeySpec{
		ID:  "mixed",
		Key: "mixed-local-key",
		ModelRouting: &modelRoutingSpec{
			DefaultRoute:  "oauth",
			FailurePolicy: "strict",
			Routes: []modelRouteSpec{{
				ID:                "route-grok",
				Namespace:         "grok",
				ProviderAccountID: "codex-grok-test",
				NativeProvider:    "xai",
				ProviderGateway: &providerGatewaySpec{
					BaseURL:        "https://cli-chat-proxy.grok.com/v1",
					UpstreamModel:  "grok-4.6",
					UpstreamModels: []string{"grok-4.6"},
					WireAPI:        "responses",
				},
			}},
		},
	}

	route, upstreamModel, status := resolveModelRoutingRoute(spec, "grok/grok-4.6")
	if status != "native" {
		t.Fatalf("status = %q, want native", status)
	}
	if route == nil || route.NativeProvider != "xai" {
		t.Fatalf("native provider route missing: %#v", route)
	}
	if upstreamModel != "grok-4.6" {
		t.Fatalf("upstream model = %q", upstreamModel)
	}
}

func TestExecutionProvidersCoversCodexAndXAI(t *testing.T) {
	providers := executionProviders()
	if len(providers) != 2 || providers[0] != "codex" || providers[1] != "xai" {
		t.Fatalf("unexpected execution providers: %#v", providers)
	}
}

func TestManifestModelsForXAIAuthUseAccountModelIDs(t *testing.T) {
	account := &accountSpec{
		ID:       "codex-grok-test",
		AuthID:   "xai-codex_grok_test.json",
		Provider: "xai",
		ModelIDs: []string{"grok-4.6", "grok-4.5"},
	}
	m := &manifest{
		ModelIDs: []string{"gpt-5.5", "grok-4.6"},
		Accounts: []accountSpec{*account},
		accountByID: map[string]*accountSpec{
			account.ID: account,
		},
		accountByAuthID: map[string]*accountSpec{
			account.AuthID: account,
		},
	}
	auth := &coreauth.Auth{
		ID:       account.AuthID,
		Provider: "xai",
		Attributes: map[string]string{
			"account_id": account.ID,
		},
	}

	models := manifestModelsForAuth(m, auth)
	if len(models) != 2 {
		t.Fatalf("models = %#v", models)
	}
	if models[0].ID != "grok-4.6" || models[1].ID != "grok-4.5" {
		t.Fatalf("unexpected models: %#v", models)
	}
}
