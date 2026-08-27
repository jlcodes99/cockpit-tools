package registry

import "testing"

// TestCodebuddyModelSupportsImagesWhitelist verifies that models confirmed by
// live-backend measurement to support images (despite app.asar marking them
// text-only) report vision support via the whitelist.
func TestCodebuddyModelSupportsImagesWhitelist(t *testing.T) {
	whitelisted := []string{
		"deepseek-v4-flash",
		"deepseek-v4-pro",
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
	if !CodebuddyModelSupportsImages("DeepSeek-V4-Flash") {
		t.Error("whitelist lookup should be case-insensitive")
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
