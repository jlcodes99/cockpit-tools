package models

import "testing"

func TestDeepSeekV41ThirdPartyCatalogUsesNativeCapabilities(t *testing.T) {
	response := BuildResponseForClient([]map[string]any{{"id": "deepseek-v4.1-flash"}}, nil, false, "0.163.0")
	model := response["models"].([]map[string]any)[0]
	if model["slug"] != "deepseek-v4.1-flash" {
		t.Fatalf("upstream model identity changed: %v", model["slug"])
	}
	if intModelValue(model, "context_window") != 1000000 || intModelValue(model, "max_context_window") != 1000000 {
		t.Fatalf("context = %v/%v, want 1000000/1000000", model["context_window"], model["max_context_window"])
	}
	if intModelValue(model, "auto_compact_token_limit") != 900000 {
		t.Fatalf("compact limit = %v, want 900000", model["auto_compact_token_limit"])
	}
	levels := model["supported_reasoning_levels"].([]any)
	want := []string{"low", "medium", "high", "xhigh", "max"}
	if len(levels) != len(want) {
		t.Fatalf("reasoning levels = %#v, want low/medium/high/xhigh/max", levels)
	}
	for i, effort := range want {
		if got := levels[i].(map[string]any)["effort"]; got != effort {
			t.Fatalf("reasoning level %d = %v, want %s", i, got, effort)
		}
	}
	if model["default_reasoning_level"] != "high" || model["tool_mode"] != nil || model["use_responses_lite"] != false {
		t.Fatalf("incorrect provider reasoning or execution metadata: default=%v tool_mode=%v lite=%v", model["default_reasoning_level"], model["tool_mode"], model["use_responses_lite"])
	}
}

func TestDeepSeekV41LegacyClientFiltersUnsupportedMax(t *testing.T) {
	response := BuildResponseForClient([]map[string]any{{"id": "deepseek-v4.1-flash"}}, nil, false, "0.143.0")
	model := response["models"].([]map[string]any)[0]
	levels := model["supported_reasoning_levels"].([]any)
	if len(levels) != 4 || levels[0].(map[string]any)["effort"] != "low" || levels[3].(map[string]any)["effort"] != "xhigh" {
		t.Fatalf("legacy reasoning levels = %#v, want low/medium/high/xhigh", levels)
	}
}

func TestProviderOverridesKeepCanonicalTemplatesAndInheritSchema(t *testing.T) {
	codexClientModelTemplatesMu.Lock()
	loaded, revision := codexClientModelTemplatesLoaded, codexClientModelTemplatesRevision
	templates, fallback, previousErr := codexClientModelTemplates, codexClientDefaultTemplate, codexClientModelTemplatesErr
	codexClientModelTemplatesLoaded = false
	codexClientModelTemplatesMu.Unlock()
	t.Cleanup(func() {
		codexClientModelTemplatesMu.Lock()
		defer codexClientModelTemplatesMu.Unlock()
		codexClientModelTemplatesLoaded, codexClientModelTemplatesRevision = loaded, revision
		codexClientModelTemplates, codexClientDefaultTemplate, codexClientModelTemplatesErr = templates, fallback, previousErr
	})

	raw := []byte(`{"models":[{"slug":"gpt-5.6-sol","context_window":272000,"tool_mode":"code_mode_only","apply_patch_tool_type":"freeform"}],"model_overrides":[{"slug":"gpt-5.6-sol","context_window":1},{"slug":"deepseek-v4.1-flash","context_window":1000000,"tool_mode":null},{"slug":"","context_window":1}]}`)
	resolved, defaultTemplate, err := loadCodexClientModelTemplatesSnapshot(raw, 1)
	if err != nil {
		t.Fatal(err)
	}
	if len(resolved) != 2 || intModelValue(resolved["gpt-5.6-sol"], "context_window") != 272000 || intModelValue(defaultTemplate, "context_window") != 272000 {
		t.Fatal("provider overrides changed a canonical template or accepted an empty ID")
	}
	deepseek := resolved["deepseek-v4.1-flash"]
	if intModelValue(deepseek, "context_window") != 1000000 || deepseek["tool_mode"] != nil || deepseek["apply_patch_tool_type"] != "freeform" {
		t.Fatal("provider override did not inherit the schema and replace declared capabilities")
	}
}
