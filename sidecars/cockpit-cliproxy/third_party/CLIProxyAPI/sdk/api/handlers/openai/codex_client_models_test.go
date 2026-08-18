package openai

import (
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
)

func TestBuildCodexClientModelsPreserves56Capabilities(t *testing.T) {
	models := buildCodexClientModels([]map[string]any{
		{"id": "gpt-5.4-mini"},
		{"id": "gpt-5.5"},
		{"id": "gpt-5.6-terra"},
		{"id": "gpt-5.4"},
		{"id": "gpt-5.6-luna"},
		{"id": "gpt-5.6-sol"},
	}, nil, false)
	wantOrder := []string{
		"gpt-5.6-sol",
		"gpt-5.6-terra",
		"gpt-5.6-luna",
		"gpt-5.5",
		"gpt-5.4",
		"gpt-5.4-mini",
	}
	for index, want := range wantOrder {
		if got := stringModelValue(models[index], "slug"); got != want {
			t.Fatalf("model[%d] = %q, want %q", index, got, want)
		}
	}

	bySlug := make(map[string]map[string]any, len(models))
	for _, model := range models {
		bySlug[stringModelValue(model, "slug")] = model
	}

	for _, testCase := range []struct {
		slug          string
		defaultEffort string
		supportsUltra bool
	}{
		{slug: "gpt-5.6-sol", defaultEffort: "low", supportsUltra: true},
		{slug: "gpt-5.6-terra", defaultEffort: "medium", supportsUltra: true},
		{slug: "gpt-5.6-luna", defaultEffort: "medium", supportsUltra: false},
	} {
		model := bySlug[testCase.slug]
		if model == nil {
			t.Fatalf("expected model %s", testCase.slug)
		}
		if supported, ok := model["supports_parallel_tool_calls"].(bool); !ok || !supported {
			t.Fatalf(
				"%s supports_parallel_tool_calls = %#v, want true",
				testCase.slug,
				model["supports_parallel_tool_calls"],
			)
		}
		if got := stringModelValue(model, "default_reasoning_level"); got != testCase.defaultEffort {
			t.Fatalf("%s default reasoning = %q, want %q", testCase.slug, got, testCase.defaultEffort)
		}
		if got := intModelValue(model, "context_window"); got != 372000 {
			t.Fatalf("%s context_window = %d, want 372000", testCase.slug, got)
		}
		if got := intModelValue(model, "max_context_window"); got != 372000 {
			t.Fatalf("%s max_context_window = %d, want 372000", testCase.slug, got)
		}
		if got := stringModelValue(model, "minimal_client_version"); got != "0.144.0" {
			t.Fatalf("%s minimal_client_version = %q, want 0.144.0", testCase.slug, got)
		}
		if got, ok := model["supports_search_tool"].(bool); !ok || !got {
			t.Fatalf("%s supports_search_tool = %#v, want true", testCase.slug, model["supports_search_tool"])
		}
		if got := stringModelValue(model, "tool_mode"); got != "code_mode_only" {
			t.Fatalf("%s tool_mode = %q, want code_mode_only", testCase.slug, got)
		}
		if got, ok := model["use_responses_lite"].(bool); !ok || !got {
			t.Fatalf("%s use_responses_lite = %#v, want true", testCase.slug, model["use_responses_lite"])
		}
		if got := stringModelValue(model, "shell_type"); got != "shell_command" {
			t.Fatalf("%s shell_type = %q, want shell_command", testCase.slug, got)
		}
		if got := stringModelValue(model, "apply_patch_tool_type"); got != "freeform" {
			t.Fatalf("%s apply_patch_tool_type = %q, want freeform", testCase.slug, got)
		}
		levels, ok := model["supported_reasoning_levels"].([]any)
		if !ok {
			t.Fatalf("%s reasoning levels = %#v", testCase.slug, model["supported_reasoning_levels"])
		}
		hasMax := false
		hasUltra := false
		for _, rawLevel := range levels {
			level, _ := rawLevel.(map[string]any)
			switch stringModelValue(level, "effort") {
			case "max":
				hasMax = true
			case "ultra":
				hasUltra = true
			}
		}
		if !hasMax || hasUltra != testCase.supportsUltra {
			t.Fatalf("%s reasoning levels max=%v ultra=%v", testCase.slug, hasMax, hasUltra)
		}
		speedTiers, ok := model["additional_speed_tiers"].([]any)
		if !ok || len(speedTiers) != 1 || speedTiers[0] != "fast" {
			t.Fatalf("%s speed tiers = %#v", testCase.slug, model["additional_speed_tiers"])
		}
		serviceTiers, ok := model["service_tiers"].([]any)
		if !ok || len(serviceTiers) != 1 {
			t.Fatalf("%s service tiers = %#v", testCase.slug, model["service_tiers"])
		}
	}
}

func TestCodexClientModelsResponse_DisablesSearchToolForSynthesizedModels(t *testing.T) {
	resp := CodexClientModelsResponse([]map[string]any{
		{"id": "custom-openai-compatible-model"},
		{"id": "gpt-5.5"},
	})
	models, ok := resp["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models type = %T, want []map[string]any", resp["models"])
	}

	bySlug := make(map[string]map[string]any, len(models))
	for _, model := range models {
		bySlug[stringModelValue(model, "slug")] = model
	}

	custom := bySlug["custom-openai-compatible-model"]
	if custom == nil {
		t.Fatal("expected synthesized custom model entry")
	}
	if got, ok := custom["supports_search_tool"].(bool); !ok || got {
		t.Fatalf("custom supports_search_tool = %#v, want false", custom["supports_search_tool"])
	}

	official := bySlug["gpt-5.5"]
	if official == nil {
		t.Fatal("expected official template model entry")
	}
	if got, ok := official["supports_search_tool"].(bool); !ok || !got {
		t.Fatalf("official supports_search_tool = %#v, want true", official["supports_search_tool"])
	}
}

func TestCodexClientModelsResponse_RequiresTemplateAndCodexProvidersForSearchTool(t *testing.T) {
	providers := map[string][]string{
		"new-codex-model": {"codex"},
		"gpt-5.5":         {"openai-compatible-deepseek"},
		"gpt-5.4":         {"codex", "xai"},
		"gpt-5.6-sol":     {"codex"},
	}
	resp := codexClientModelsResponse([]map[string]any{
		{"id": "new-codex-model"},
		{"id": "gpt-5.5"},
		{"id": "gpt-5.4"},
		{"id": "gpt-5.6-sol"},
	}, func(id string) []string {
		return providers[id]
	}, false)
	models, ok := resp["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models type = %T, want []map[string]any", resp["models"])
	}

	bySlug := make(map[string]map[string]any, len(models))
	for _, model := range models {
		bySlug[stringModelValue(model, "slug")] = model
	}

	if got, ok := bySlug["new-codex-model"]["supports_search_tool"].(bool); !ok || got {
		t.Fatalf("non-template supports_search_tool = %#v, want false", bySlug["new-codex-model"]["supports_search_tool"])
	}
	if got, ok := bySlug["gpt-5.5"]["supports_search_tool"].(bool); !ok || got {
		t.Fatalf("mixed/non-codex provider supports_search_tool = %#v, want false", bySlug["gpt-5.5"]["supports_search_tool"])
	}
	if got, ok := bySlug["gpt-5.4"]["supports_search_tool"].(bool); !ok || got {
		t.Fatalf("mixed providers supports_search_tool = %#v, want false", bySlug["gpt-5.4"]["supports_search_tool"])
	}
	if got, ok := bySlug["gpt-5.6-sol"]["supports_search_tool"].(bool); !ok || !got {
		t.Fatalf("codex-only template supports_search_tool = %#v, want true", bySlug["gpt-5.6-sol"]["supports_search_tool"])
	}
}

func TestCodexClientModelsResponse_Gates56ResponsesLiteByProvider(t *testing.T) {
	providers := map[string][]string{
		"gpt-5.6-sol":   {"codex"},
		"gpt-5.6-terra": {"openai-compatibility"},
		"gpt-5.6-luna":  {"codex", "openai-compatibility"},
	}
	response := CodexClientModelsResponseWithProviders([]map[string]any{
		{"id": "gpt-5.6-sol"},
		{"id": "gpt-5.6-terra"},
		{"id": "gpt-5.6-luna"},
	}, func(id string) []string {
		return providers[id]
	})
	models, ok := response["models"].([]map[string]any)
	if !ok {
		t.Fatalf("models type = %T, want []map[string]any", response["models"])
	}
	bySlug := make(map[string]map[string]any, len(models))
	for _, model := range models {
		bySlug[stringModelValue(model, "slug")] = model
	}

	if got, ok := bySlug["gpt-5.6-sol"]["use_responses_lite"].(bool); !ok || !got {
		t.Fatalf("official Codex OAuth use_responses_lite = %#v, want true", bySlug["gpt-5.6-sol"]["use_responses_lite"])
	}
	for _, slug := range []string{"gpt-5.6-terra", "gpt-5.6-luna"} {
		if got, ok := bySlug[slug]["use_responses_lite"].(bool); !ok || got {
			t.Fatalf("%s custom-provider use_responses_lite = %#v, want false", slug, bySlug[slug]["use_responses_lite"])
		}
	}
}

func TestCodexClientModelsResponseMultiAgentV2FollowsConfig(t *testing.T) {
	modelID := "codex-client-multi-agent-v2-test"
	clientID := "codex-client-multi-agent-v2-test-client"
	modelRegistry := registry.GetGlobalRegistry()
	modelRegistry.RegisterClient(clientID, "openai-compatibility", []*registry.ModelInfo{{ID: modelID}})
	t.Cleanup(func() {
		modelRegistry.UnregisterClient(clientID)
	})

	base := handlers.NewBaseAPIHandlers(&config.SDKConfig{}, nil)
	handler := NewOpenAIAPIHandler(base)
	for _, tt := range []struct {
		name    string
		enabled bool
	}{
		{name: "disabled", enabled: false},
		{name: "enabled", enabled: true},
	} {
		t.Run(tt.name, func(t *testing.T) {
			base.Cfg.CodexOptimizeMultiAgentV2 = tt.enabled
			response := handler.codexClientModelsResponse()
			models, ok := response["models"].([]map[string]any)
			if !ok {
				t.Fatalf("models type = %T, want []map[string]any", response["models"])
			}
			var entry map[string]any
			for _, model := range models {
				slug, _ := model["slug"].(string)
				if slug == modelID {
					entry = model
					break
				}
			}
			if entry == nil {
				t.Fatalf("missing synthesized model %q", modelID)
			}
			value, exists := entry["multi_agent_version"]
			if tt.enabled {
				if !exists || value != "v2" {
					t.Fatalf("multi_agent_version = %#v, want v2", value)
				}
				return
			}
			if !exists || value != nil {
				t.Fatalf("multi_agent_version = %#v, want preserved null", value)
			}
		})
	}
}
