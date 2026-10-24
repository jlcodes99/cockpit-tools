package main

import (
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

// Grok 账号在宿主 manifest 里 provider 写作 "grok"，sidecar 内部统一用 "xai"。
// 归一化缺失会把 xai-only 模型注册到 Codex 账号上：这些账号随后进入候选，
// 又因为不承接该模型、不在 API Key 作用域内而被过滤，最终报
// 「账号池没有可用账号：候选 1 个，不可用 0 个…」。
func TestGrokProviderAccountsMarkXaiOnlyModels(t *testing.T) {
	m := &manifest{
		ModelIDs: []string{"grok-4.6", "gpt-5.2-codex"},
		Accounts: []accountSpec{
			{
				ID:       "codex_grok_test",
				Provider: "grok",
				AuthID:   "xai-codex_grok_test.json",
				ModelIDs: []string{"grok-4.6"},
			},
		},
	}
	m.accountByID = map[string]*accountSpec{"codex_grok_test": &m.Accounts[0]}
	m.accountByAuthID = map[string]*accountSpec{"xai-codex_grok_test.json": &m.Accounts[0]}

	only := xaiOnlyModelIDs(m)
	if _, ok := only["grok-4.6"]; !ok {
		t.Fatalf("grok-4.6 应被识别为 xai-only，实际 %v", only)
	}

	codexAuth := &coreauth.Auth{ID: "codex_oauth_test.json", Provider: "codex"}
	for _, model := range manifestModelsForAuth(m, codexAuth) {
		if model != nil && model.ID == "grok-4.6" {
			t.Fatalf("Codex 账号不应承接 xai-only 模型 grok-4.6")
		}
	}

	grokAuth := &coreauth.Auth{ID: "xai-codex_grok_test.json", Provider: "xai"}
	found := false
	for _, model := range manifestModelsForAuth(m, grokAuth) {
		if model != nil && model.ID == "grok-4.6" {
			found = true
		}
	}
	if !found {
		t.Fatalf("Grok 账号应承接 grok-4.6")
	}

	if !sidecarOAuthProviderSupported("grok") {
		t.Fatalf("manifest 里的 grok provider 必须被识别为受支持的 xai provider")
	}
}

func TestGrokCustomUpstreamRegistersOnlyForItsBoundAuth(t *testing.T) {
	for _, catalog := range [][]string{nil, {"grok-4.6"}} {
		account := &accountSpec{ID: "custom-grok", AuthID: "custom-grok.json", Provider: "grok", ModelIDs: catalog}
		other := &accountSpec{ID: "other-grok", AuthID: "other-grok.json", Provider: "grok", ModelIDs: []string{"grok-4.6"}}
		m := &manifest{accountByAuthID: map[string]*accountSpec{account.AuthID: account, other.AuthID: other},
			APIKeys: []apiKeySpec{{ModelRouting: &modelRoutingSpec{Automatic: true, Routes: []modelRouteSpec{{
				ProviderAccountID: account.ID, NativeProvider: "xai",
				Models: []modelRouteModelSpec{{ClientModel: "custom", UpstreamModel: "grok-new-upstream"}},
			}}}}}}
		for _, auth := range []*coreauth.Auth{{ID: account.AuthID, Provider: "xai"}, {ID: other.AuthID, Provider: "xai"}} {
			registry.GetGlobalRegistry().RegisterClient(auth.ID, auth.Provider, manifestModelsForAuth(m, auth))
			if got := registry.GetGlobalRegistry().ClientSupportsModel(auth.ID, "grok-new-upstream"); got != (auth.ID == account.AuthID) {
				t.Fatalf("custom upstream support for %s = %v", auth.ID, got)
			}
			registry.GetGlobalRegistry().UnregisterClient(auth.ID)
		}
	}
}
