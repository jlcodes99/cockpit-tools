package main

import (
	"context"
	"strings"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func imageAccountsTestSpec(withImageAccounts bool) *apiKeySpec {
	spec := &apiKeySpec{
		ID:         "provider-gateway-key",
		Key:        "gateway-key",
		Enabled:    true,
		AccountIDs: []string{"deepseek-account"},
		ProviderGateway: &providerGatewaySpec{
			BaseURL:       "https://api.deepseek.com",
			APIKey:        "sk-test",
			UpstreamModel: "deepseek-flash",
			UpstreamModels: []string{
				"deepseek-flash",
			},
			WireAPI: "responses",
		},
	}
	if withImageAccounts {
		spec.ImageGenerationAccountIDs = []string{"gpt-image-account"}
	}
	return spec
}

func TestProviderGatewayAllowsImageModelOnlyWithImageAccounts(t *testing.T) {
	m := &manifest{
		ModelIDs:             []string{"gpt-5.5"},
		ImageGenerationModel: defaultImagesToolModel,
	}

	withAccounts := imageAccountsTestSpec(true)
	if !validateClientModelVisible(m, withAccounts, defaultImagesToolModel, defaultImagesToolModel) {
		t.Fatal("expected image model to be visible when image generation accounts are configured")
	}

	withoutAccounts := imageAccountsTestSpec(false)
	if validateClientModelVisible(m, withoutAccounts, defaultImagesToolModel, defaultImagesToolModel) {
		t.Fatal("expected image model to stay hidden without image generation accounts")
	}
}

func TestVisibleModelsIncludeImageModelForImageAccounts(t *testing.T) {
	m := &manifest{
		ModelIDs:             []string{"gpt-5.5"},
		ImageGenerationModel: defaultImagesToolModel,
	}
	models := visibleModelsForAPIKey(m, imageAccountsTestSpec(true))
	found := false
	for _, model := range models {
		if strings.EqualFold(model, defaultImagesToolModel) {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected %s in visible models, got %#v", defaultImagesToolModel, models)
	}
}

func TestImageRequestsUseImageGenerationAccounts(t *testing.T) {
	chatAccount := &accountSpec{
		ID:                    "deepseek-account",
		AuthID:                "deepseek.json",
		AuthKind:              "api_key",
		ImageGenerationPolicy: "disabled",
	}
	imageAccount := &accountSpec{
		ID:       "gpt-image-account",
		AuthID:   "gpt.json",
		AuthKind: "oauth",
		PlanType: "plus",
	}
	selector := &cockpitSelector{
		manifest: &manifest{accountByAuthID: map[string]*accountSpec{
			"deepseek.json": chatAccount,
			"gpt.json":      imageAccount,
		}},
	}
	spec := imageAccountsTestSpec(true)
	auths := []*coreauth.Auth{{ID: "deepseek.json"}, {ID: "gpt.json"}}

	imageCtx := context.WithValue(context.Background(), clientAPIKeyContextKey, spec)
	imageCtx = context.WithValue(imageCtx, requestKindContextKey, "image_generation")
	scoped := selector.filterAuthsForAPIKeyScope(imageCtx, auths)
	if len(scoped) != 1 || scoped[0].ID != "gpt.json" {
		t.Fatalf("image request should be scoped to image accounts, got %#v", scoped)
	}

	chatCtx := context.WithValue(context.Background(), clientAPIKeyContextKey, spec)
	chatScoped := selector.filterAuthsForAPIKeyScope(chatCtx, auths)
	if len(chatScoped) != 1 || chatScoped[0].ID != "deepseek.json" {
		t.Fatalf("chat request should stay on the chat account, got %#v", chatScoped)
	}
}
