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

// TestStripCodebuddyThinkingSuffix verifies that the official client's
// thinking-variant suffix (e.g. `-1`) is stripped so the ID matches the backend
// model ID. Without this, routing fails with auth_not_found for models like
// kimi-k3 (advertised as `kimi-k3-1` in the app.asar).
func TestStripCodebuddyThinkingSuffix(t *testing.T) {
	cases := []struct {
		in   string
		want string
	}{
		{"kimi-k3-1", "kimi-k3"},
		{"kimi-k3", "kimi-k3"},
		{"deepseek-v4-pro", "deepseek-v4-pro"},
		{"glm-5.2", "glm-5.2"},
		{"kimi-k2-0711-preview", "kimi-k2-0711-preview"},
		{"-1", "-1"}, // 剥离后为空则保留原值，避免产生空 ID
		{"", ""},
	}
	for _, tc := range cases {
		if got := stripCodebuddyThinkingSuffix(tc.in); got != tc.want {
			t.Errorf("stripCodebuddyThinkingSuffix(%q) = %q, want %q", tc.in, got, tc.want)
		}
	}
}
