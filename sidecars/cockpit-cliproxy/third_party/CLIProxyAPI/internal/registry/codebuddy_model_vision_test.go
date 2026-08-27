package registry

import "testing"

// TestCodebuddyModelSupportsImagesWhitelist verifies that models confirmed by
// live-backend measurement to support images (despite app.asar marking them
// text-only) report vision support via the whitelist.
func TestCodebuddyModelSupportsImagesWhitelist(t *testing.T) {
	whitelisted := []string{
		"glm-5.1",
		"glm-5.2",
	}
	for _, id := range whitelisted {
		if !CodebuddyModelSupportsImages(id) {
			t.Errorf("whitelisted model %q should report vision support", id)
		}
	}
}

// TestCodebuddyModelSupportsImagesCaseInsensitive verifies the whitelist is
// case-insensitive.
func TestCodebuddyModelSupportsImagesCaseInsensitive(t *testing.T) {
	if !CodebuddyModelSupportsImages("GLM-5.1") {
		t.Error("whitelist lookup should be case-insensitive")
	}
}

// TestCodebuddyModelSupportsImagesDeepSeekNotWhitelisted verifies that
// deepseek-v4-flash / deepseek-v4-pro no longer report native vision support:
// live testing showed the backend returns a refusal text for them, so the
// vision-proxy layer (preprocess) must handle their image inputs instead.
func TestCodebuddyModelSupportsImagesDeepSeekNotWhitelisted(t *testing.T) {
	notWhitelisted := []string{
		"deepseek-v4-flash",
		"deepseek-v4-pro",
	}
	for _, id := range notWhitelisted {
		if CodebuddyModelSupportsImages(id) {
			t.Errorf("model %q should NOT report native vision support (backend returns refusal)", id)
		}
	}
}

// TestCodebuddyModelSupportsImagesFakeVision verifies that models which return
// a "model does not support images" refusal text (or unknown/empty IDs) do not
// report vision support, so the vision-proxy layer still routes them.
func TestCodebuddyModelSupportsImagesFakeVision(t *testing.T) {
	fake := []string{
		"hunyuan-2.0-instruct",
		"hunyuan-2.0-thinking",
		"unknown-model",
		"",
	}
	for _, id := range fake {
		if CodebuddyModelSupportsImages(id) {
			t.Errorf("model %q should NOT report vision support", id)
		}
	}
}

// TestCodebuddyModelMaxCompletionTokens verifies the max-completion ceiling
// lookup falls back to the shared default for unknown/empty model IDs.
func TestCodebuddyModelMaxCompletionTokens(t *testing.T) {
	if got := CodebuddyModelMaxCompletionTokens(""); got != CodebuddyMaxCompletionTokensDefault {
		t.Fatalf("empty model = %d, want %d", got, CodebuddyMaxCompletionTokensDefault)
	}
	if got := CodebuddyModelMaxCompletionTokens("unknown-model"); got != CodebuddyMaxCompletionTokensDefault {
		t.Fatalf("unknown model = %d, want %d", got, CodebuddyMaxCompletionTokensDefault)
	}
	// A known CodeBuddy model should resolve to a positive ceiling (32768).
	if got := CodebuddyModelMaxCompletionTokens("deepseek-v4-pro"); got <= 0 {
		t.Fatalf("known model = %d, want > 0", got)
	}
}
