package registry

import "testing"

// TestExtractCodebuddyModelsFromLocalClient verifies the extractor against the
// locally installed official client when present. It is skipped on machines
// without WorkBuddy/CodeBuddy installed.
func TestExtractCodebuddyModelsFromLocalClient(t *testing.T) {
	asarPath := findCodebuddyAsarPath()
	if asarPath == "" {
		t.Skip("official WorkBuddy/CodeBuddy client not installed; skipping")
	}

	models, err := extractCodebuddyModels(asarPath)
	if err != nil {
		t.Fatalf("extractCodebuddyModels(%s): %v", asarPath, err)
	}
	if len(models) == 0 {
		t.Fatalf("extracted 0 models from %s", asarPath)
	}

	seen := make(map[string]bool, len(models))
	hasKimiK3 := false
	for _, m := range models {
		if m == nil || m.ID == "" {
			t.Errorf("extracted model with empty ID")
			continue
		}
		if !isValidModelID(m.ID) {
			t.Errorf("extracted invalid model ID %q", m.ID)
		}
		if seen[m.ID] {
			t.Errorf("duplicate model ID %q", m.ID)
		}
		seen[m.ID] = true

		if m.ID == "kimi-k3" {
			hasKimiK3 = true
			if m.DisplayName == "" {
				t.Errorf("kimi-k3 has empty display name")
			}
			if m.Type != "codebuddy" {
				t.Errorf("kimi-k3 has type %q, want codebuddy", m.Type)
			}
		}
	}

	if !hasKimiK3 {
		t.Errorf("expected kimi-k3 in extracted models; got %d models", len(models))
	}
	t.Logf("extracted %d unique models from %s", len(models), asarPath)
}
