package registry

import "testing"

func TestGetStaticModelDefinitionsByChannelSupportsGeminiInteractions(t *testing.T) {
	models := GetStaticModelDefinitionsByChannel("gemini-interactions")
	if len(models) == 0 {
		t.Fatal("GetStaticModelDefinitionsByChannel(gemini-interactions) returned no models")
	}
}

func TestModelOverrideHeadersFromEmbeddedModels(t *testing.T) {
	const wantUA = "codex-tui/0.144.0 (Mac OS 26.5.1; arm64) iTerm.app/3.6.11 (codex-tui; 0.144.0)"
	got := ModelOverrideHeaders("gpt-5.6-luna")
	if got == nil {
		t.Fatal("ModelOverrideHeaders(gpt-5.6-luna) = nil, want headers")
	}
	if got["user-agent"] != wantUA {
		t.Fatalf("user-agent = %q, want %q", got["user-agent"], wantUA)
	}
	if got := ModelOverrideHeaders("gpt-5.4"); got != nil {
		t.Fatalf("ModelOverrideHeaders(gpt-5.4) = %#v, want nil", got)
	}
}

func TestGeminiVertexModelsUseFlashLiteReleaseID(t *testing.T) {
	const releaseID = "gemini-3.1-flash-lite"
	const previewID = releaseID + "-preview"

	for _, model := range GetGeminiVertexModels() {
		if model == nil {
			continue
		}
		if model.ID == previewID {
			t.Fatalf("Vertex model ID = %q, want release ID %q", model.ID, releaseID)
		}
		if model.ID == releaseID {
			return
		}
	}

	t.Fatalf("Vertex models do not contain %q", releaseID)
}

func TestWithXAIBuiltinsIncludesImage20(t *testing.T) {
	models := WithXAIBuiltins(nil)
	for _, model := range models {
		if model != nil && model.ID == xaiBuiltinImage20ModelID {
			if model.Created != 1786060800 {
				t.Fatalf("created = %d, want 1786060800 (2026-08-07)", model.Created)
			}
			return
		}
	}
	t.Fatalf("expected xAI builtin model %s", xaiBuiltinImage20ModelID)
}

func TestPaidCodexModelsIncludeGPT6FamilyButFreeDoesNot(t *testing.T) {
	wantOrder := []string{
		codexBuiltinGPT6AstraModelID,
		codexBuiltinGPT6SolModelID,
		codexBuiltinGPT6LunaModelID,
	}
	wantDisplayNames := map[string]string{
		codexBuiltinGPT6AstraModelID: "GPT-6 Astra",
		codexBuiltinGPT6SolModelID:   "GPT-6 Sol",
		codexBuiltinGPT6LunaModelID:  "GPT-6 Luna",
	}

	for _, models := range [][]*ModelInfo{
		GetCodexTeamModels(),
		GetCodexPlusModels(),
		GetCodexProModels(),
	} {
		if len(models) < len(wantOrder) {
			t.Fatalf("paid Codex models = %d entries, want at least %d", len(models), len(wantOrder))
		}
		for i, wantID := range wantOrder {
			if models[i] == nil || models[i].ID != wantID {
				t.Fatalf("paid Codex model %d = %#v, want %s", i, models[i], wantID)
			}
		}

		byID := make(map[string]*ModelInfo, len(models))
		for _, model := range models {
			if model != nil {
				byID[model.ID] = model
			}
		}
		for _, modelID := range wantOrder {
			model := byID[modelID]
			if model == nil {
				t.Fatalf("paid Codex models do not contain %s", modelID)
			}
			if model.ContextLength != 256000 || model.MaxCompletionTokens != 128000 {
				t.Fatalf("%s limits = %d/%d, want 256000/128000", modelID, model.ContextLength, model.MaxCompletionTokens)
			}
			if model.DisplayName != wantDisplayNames[modelID] {
				t.Fatalf("%s display name = %q, want %q", modelID, model.DisplayName, wantDisplayNames[modelID])
			}
		}

		astra := byID[codexBuiltinGPT6AstraModelID]
		if astra.Thinking == nil || len(astra.Thinking.Levels) != 6 || astra.Thinking.Levels[4] != "max" || astra.Thinking.Levels[5] != "ultra" {
			t.Fatalf("Astra reasoning levels = %#v", astra.Thinking)
		}
		sol := byID[codexBuiltinGPT6SolModelID]
		if sol.Thinking == nil || len(sol.Thinking.Levels) != 6 || sol.Thinking.Levels[4] != "max" || sol.Thinking.Levels[5] != "ultra" {
			t.Fatalf("Sol reasoning levels = %#v", sol.Thinking)
		}
		luna := byID[codexBuiltinGPT6LunaModelID]
		if luna.Thinking == nil || len(luna.Thinking.Levels) != 5 || luna.Thinking.Levels[4] != "max" {
			t.Fatalf("Luna reasoning levels = %#v", luna.Thinking)
		}
		for _, level := range luna.Thinking.Levels {
			if level == "ultra" {
				t.Fatalf("Luna reasoning levels = %#v, want no ultra", luna.Thinking)
			}
		}
	}

	for _, model := range GetCodexFreeModels() {
		if model == nil {
			continue
		}
		for _, modelID := range wantOrder {
			if model.ID == modelID {
				t.Fatalf("free Codex models should not advertise %s before entitlement rollout", modelID)
			}
		}
	}
}

func TestLookupStaticModelInfoFallsBackToShippedGPT6Builtins(t *testing.T) {
	for _, modelID := range []string{
		codexBuiltinGPT6AstraModelID,
		codexBuiltinGPT6SolModelID,
		codexBuiltinGPT6LunaModelID,
	} {
		model := LookupStaticModelInfo(modelID)
		if model == nil {
			t.Fatalf("LookupStaticModelInfo(%s) = nil, want shipped builtin", modelID)
		}
		if model.ID != modelID {
			t.Fatalf("LookupStaticModelInfo(%s).ID = %s, want %s", modelID, model.ID, modelID)
		}
		if model.ContextLength != 256000 || model.MaxCompletionTokens != 128000 {
			t.Fatalf("%s limits = %d/%d, want 256000/128000", modelID, model.ContextLength, model.MaxCompletionTokens)
		}
	}
	if model := LookupStaticModelInfo("gpt-6-unknown"); model != nil {
		t.Fatalf("LookupStaticModelInfo(gpt-6-unknown) = %#v, want nil", model)
	}
}

func TestWithXAIBuiltinsIncludesVideo15GAAndPreviewAlias(t *testing.T) {
	models := WithXAIBuiltins(nil)
	foundGA := false
	foundPreviewAlias := false

	for _, model := range models {
		if model == nil {
			continue
		}
		if model.ID == xaiBuiltinVideo15ModelID {
			foundGA = true
		}
		if model.ID == xaiBuiltinVideo15PreviewID {
			foundPreviewAlias = true
		}
	}

	if !foundGA {
		t.Fatalf("expected xAI builtin model %s", xaiBuiltinVideo15ModelID)
	}
	if !foundPreviewAlias {
		t.Fatalf("expected xAI builtin compatibility alias %s", xaiBuiltinVideo15PreviewID)
	}
}

func TestAntigravityWebSearchModelForRequiresRequestedModelCapability(t *testing.T) {
	registryRef := GetGlobalRegistry()
	registryRef.RegisterClient("test-antigravity-websearch-route", "antigravity", []*ModelInfo{
		{ID: "gemini-route-test"},
		{ID: "gemini-web-search-test", SupportsWebSearch: true},
	})
	registryRef.RegisterClient("test-gemini-websearch-route", "gemini", []*ModelInfo{
		{ID: "gemini-cross-provider-route"},
		{ID: "gemini-cross-provider-search", SupportsWebSearch: true},
	})
	t.Cleanup(func() {
		registryRef.UnregisterClient("test-antigravity-websearch-route")
		registryRef.UnregisterClient("test-gemini-websearch-route")
	})

	if got := AntigravityWebSearchModelFor("gemini-route-test"); got != "" {
		t.Fatalf("route model without web search support should not get fallback model, got %q", got)
	}
	if got := AntigravityWebSearchModelFor("gemini-route-test(high)"); got != "" {
		t.Fatalf("suffix route model without web search support should not get fallback model, got %q", got)
	}
	if got := AntigravityWebSearchModelFor("gemini-web-search-test"); got != "gemini-web-search-test" {
		t.Fatalf("AntigravityWebSearchModelFor capable model = %q, want itself", got)
	}
	if got := AntigravityWebSearchModelFor("gemini-cross-provider-route"); got != "" {
		t.Fatalf("cross-provider model should not get Antigravity web search model, got %q", got)
	}
	if got := AntigravityWebSearchModelFor("unknown-model"); got != "" {
		t.Fatalf("unknown model should not get Antigravity web search model, got %q", got)
	}
}
