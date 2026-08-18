package registry

import (
	"strings"
	"testing"
)

func TestCodexPlanModelsIncludeManaged56Models(t *testing.T) {
	for plan, models := range map[string][]*ModelInfo{
		"free": GetCodexFreeModels(),
		"team": GetCodexTeamModels(),
		"plus": GetCodexPlusModels(),
		"pro":  GetCodexProModels(),
	} {
		byID := make(map[string]*ModelInfo, len(models))
		for _, model := range models {
			if model != nil {
				byID[model.ID] = model
			}
		}
		for _, slug := range []string{"gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"} {
			model := byID[slug]
			if model == nil {
				t.Fatalf("%s plan is missing %s", plan, slug)
			}
			if model.ContextLength != 372000 {
				t.Fatalf("%s plan %s context length = %d", plan, slug, model.ContextLength)
			}
		}
	}

	models := GetCodexProModels()
	for _, model := range models {
		if model == nil || model.ID != "gpt-5.6-sol" {
			continue
		}
		if model.Thinking == nil || !containsString(model.Thinking.Levels, "ultra") {
			t.Fatalf("Sol reasoning levels = %#v", model.Thinking)
		}
		return
	}
	t.Fatal("missing Sol model")
}

func TestCodexResponsesLiteModels(t *testing.T) {
	for _, slug := range []string{"gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"} {
		if !CodexClientModelUsesResponsesLite(slug) {
			t.Fatalf("expected %s to use Responses Lite", slug)
		}
	}
	if CodexClientModelUsesResponsesLite("gpt-5.5") {
		t.Fatal("gpt-5.5 should not use Responses Lite")
	}
}

func TestCodexClientModelBaseInstructions(t *testing.T) {
	known := CodexClientModelBaseInstructions("gpt-5.6-sol")
	if len(known) < 100 || !strings.Contains(known, "Codex") {
		t.Fatalf("gpt-5.6-sol base instructions are missing or incomplete: %q", known)
	}

	fallback := CodexClientModelBaseInstructions("unknown-model")
	if fallback != CodexClientModelBaseInstructions("gpt-5.5") {
		t.Fatal("unknown model should fall back to gpt-5.5 base instructions")
	}
}

func containsString(values []string, expected string) bool {
	for _, value := range values {
		if value == expected {
			return true
		}
	}
	return false
}
