package registry

import "testing"

func TestV72157CodexImageBuiltinsIncludeAllVariants(t *testing.T) {
	models := WithCodexBuiltins(nil)
	seen := make(map[string]bool, len(models))
	for _, model := range models {
		if model != nil {
			seen[model.ID] = true
		}
	}
	for _, id := range []string{
		"gpt-image-1.5",
		"gpt-image-2",
		"gpt-image-2.5-flare",
		"gpt-image-2.5-sunburst",
		"gpt-image-2.5",
	} {
		if !seen[id] {
			t.Fatalf("missing Codex image model %s", id)
		}
	}
}
