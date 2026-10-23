package main

import (
	"slices"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestRetiredChatGPTModelCatalogPreservesExplicitProvider(t *testing.T) {
	m := &manifest{ModelIDs: []string{"gpt-5.5", "gpt-5.6-sol"}}
	if models := visibleModelsForAPIKey(m, nil); slices.Contains(models, "gpt-5.5") || !slices.Contains(models, "gpt-5.6-sol") {
		t.Fatalf("ChatGPT catalog contains retired model or lost active model: %v", models)
	}
	gateway := &apiKeySpec{ProviderGateway: &providerGatewaySpec{UpstreamModels: []string{"gpt-5.5"}}}
	if models := visibleModelsForAPIKey(m, gateway); !slices.Equal(models, []string{"gpt-5.5"}) {
		t.Fatalf("explicit API-key upstream was changed: %v", models)
	}
	for _, kind := range []string{coreauth.AuthKindOAuth, coreauth.AuthKindAPIKey} {
		auth := &coreauth.Auth{Provider: "codex", Attributes: map[string]string{coreauth.AttributeAuthKind: kind}}
		found := false
		for _, model := range manifestModelsForAuth(m, auth) {
			found = found || model.ID == "gpt-5.5"
		}
		if found != (kind == coreauth.AuthKindAPIKey) {
			t.Fatalf("auth kind %s retained retired model = %v", kind, found)
		}
	}
}

func TestRetiredChatGPTModelResolvesDeclaredAliases(t *testing.T) {
	m := &manifest{ModelAliases: []modelAliasSpec{
		{SourceModel: "gpt-5.5", Alias: "old-model"},
		{SourceModel: "deepseek-v4-flash", Alias: "gpt-5.5"},
	}}
	if !isRetiredChatGPTModel(m, "old-model") || isRetiredChatGPTModel(m, "gpt-5.5") {
		t.Fatal("retirement must follow the declared upstream instead of renaming custom providers")
	}
}
