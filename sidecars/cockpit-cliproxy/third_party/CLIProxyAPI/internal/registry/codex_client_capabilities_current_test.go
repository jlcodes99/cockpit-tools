package registry

import (
	"encoding/json"
	"testing"
)

func TestEmbeddedAstraKeepsCurrentNativeCapabilities(t *testing.T) {
	var payload codexClientModelsPayload
	if err := json.Unmarshal(embeddedCodexClientModelsJSON, &payload); err != nil {
		t.Fatal(err)
	}
	for _, model := range payload.Models {
		if model["slug"] != "gpt-6-astra" {
			continue
		}
		tools, ok := model["experimental_supported_tools"].([]any)
		if !ok || len(tools) != 2 || tools[0] != "send_user_message_async" || tools[1] != "clock" {
			t.Fatalf("native tools lost: %v", model["experimental_supported_tools"])
		}
		if model["multi_agent_reasoning_effort"] != "xhigh" || model["supports_reasoning_effort_updates"] != true {
			t.Fatal("native reasoning capabilities missing")
		}
		if model["default_reasoning_level"] != "medium" {
			t.Fatal("the existing default preference must remain unchanged")
		}
		return
	}
	t.Fatal("Astra model is missing")
}

func TestEmbeddedCurrentModelsKeepNodeReviewRequirement(t *testing.T) {
	var payload codexClientModelsPayload
	if err := json.Unmarshal(embeddedCodexClientModelsJSON, &payload); err != nil {
		t.Fatal(err)
	}
	found := 0
	for _, model := range payload.Models {
		if model["slug"] == "gpt-6-astra" || model["slug"] == "gpt-6-sol" {
			found++
			if model["node_repl_auto_review_required"] != true {
				t.Fatalf("review requirement lost for %v", model["slug"])
			}
		}
	}
	if found != 2 {
		t.Fatalf("reviewed model entries missing: %d", found)
	}
}
