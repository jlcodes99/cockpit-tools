package handlers

import (
	"encoding/json"
	"testing"
)

func TestBuildChatProbeBody_ReasoningEffortUsesProviderCompatibleValue(t *testing.T) {
	bodyBytes := buildChatProbeBody("test-model")

	var body map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &body); err != nil {
		t.Fatalf("unmarshal body failed: %v", err)
	}
	if body["reasoning_effort"] != "low" {
		t.Fatalf("reasoning_effort=%v, want low", body["reasoning_effort"])
	}
}
