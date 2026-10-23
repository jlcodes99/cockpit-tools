package models

// ConservativeFallbackTemplate uses the current default's request schema while
// retaining the established capabilities of models without a dedicated catalog
// template. A newer default must not silently enable client features for them.
func ConservativeFallbackTemplate(template map[string]any) map[string]any {
	entry := cloneCodexClientModelMap(template)
	entry["multi_agent_version"] = nil
	entry["minimal_client_version"] = "0.124.0"
	entry["tool_mode"] = nil
	entry["use_responses_lite"] = false
	entry["include_skills_usage_instructions"] = true
	entry["default_reasoning_level"] = "medium"
	levels := make([]any, 0, 4)
	if declared, ok := entry["supported_reasoning_levels"].([]any); ok {
		for _, raw := range declared {
			level, ok := raw.(map[string]any)
			if !ok {
				continue
			}
			switch stringModelValue(level, "effort") {
			case "low", "medium", "high", "xhigh":
				levels = append(levels, level)
			}
		}
	}
	entry["supported_reasoning_levels"] = levels
	if window, exists := entry["context_window"]; exists {
		entry["max_context_window"] = window
	}
	return entry
}
