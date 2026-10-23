package models

import "testing"

func TestGrok47ClientCatalogUsesProviderCapabilities(t *testing.T) {
	response := BuildResponse([]map[string]any{{"id": "grok-4.7"}}, nil, false)
	models := response["models"].([]map[string]any)
	if len(models) != 1 {
		t.Fatalf("model count = %d", len(models))
	}
	model := models[0]
	if intModelValue(model, "context_window") != 500000 || intModelValue(model, "max_context_window") != 500000 {
		t.Fatalf("incorrect context: %#v", model)
	}
	if model["default_reasoning_level"] != "high" {
		t.Fatalf("reasoning default = %v, want high", model["default_reasoning_level"])
	}
	levels := model["supported_reasoning_levels"].([]any)
	if len(levels) != 4 || levels[3].(map[string]any)["effort"] != "xhigh" {
		t.Fatalf("reasoning levels = %#v", levels)
	}
	modalities := model["input_modalities"].([]any)
	if len(modalities) != 2 || modalities[1] != "image" {
		t.Fatalf("modalities = %#v", modalities)
	}
}
