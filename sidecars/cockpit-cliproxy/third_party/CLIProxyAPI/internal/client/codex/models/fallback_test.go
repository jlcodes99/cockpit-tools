package models

import (
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
)

func TestUnknownModelsKeepConservativeCapabilitiesWithoutChangingSol(t *testing.T) {
	// The shipped catalog owns context limits; fallback construction must
	// preserve the current known-model limit rather than a retired snapshot.
	template, ok := registry.GetCodexModelContextTemplate("gpt-5.6-sol")
	if !ok || template.MaxContextWindow <= 0 {
		t.Fatal("known Sol context template is unavailable")
	}
	for _, enabled := range []bool{false, true} {
		response := BuildResponse([]map[string]any{
			{"id": "vendor-unknown"},
			{"id": "gpt-5.6-sol"},
			{"id": "vendor-explicit", "thinking": map[string]any{"levels": []string{"medium", "max"}}},
		}, nil, enabled)
		byID := make(map[string]map[string]any)
		for _, model := range response["models"].([]map[string]any) {
			byID[model["slug"].(string)] = model
		}
		unknown := byID["vendor-unknown"]
		if unknown["minimal_client_version"] != "0.124.0" || unknown["use_responses_lite"] != false || unknown["tool_mode"] != nil {
			t.Fatalf("unknown inherited Sol client requirements: %#v", unknown)
		}
		if enabled && unknown["multi_agent_version"] != "v2" || !enabled && unknown["multi_agent_version"] != nil {
			t.Fatalf("unknown multi-agent capability ignored opt-in %v: %#v", enabled, unknown["multi_agent_version"])
		}
		if unknown["default_reasoning_level"] != "medium" || len(unknown["supported_reasoning_levels"].([]any)) != 4 {
			t.Fatal("unknown inherited newer model reasoning capabilities")
		}
		if len(byID["vendor-explicit"]["supported_reasoning_levels"].([]any)) != 2 {
			t.Fatal("explicit thinking capabilities were overwritten")
		}
		sol := byID["gpt-5.6-sol"]
		if sol["multi_agent_version"] != "v2" || sol["minimal_client_version"] != "0.144.0" || sol["use_responses_lite"] != true || intModelValue(sol, "max_context_window") != template.MaxContextWindow {
			t.Fatalf("known Sol capabilities changed: multi_agent=%v version=%v lite=%v context=%d (catalog=%d)", sol["multi_agent_version"], sol["minimal_client_version"], sol["use_responses_lite"], intModelValue(sol, "max_context_window"), template.MaxContextWindow)
		}
	}
}
