package thinking_test

import (
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/thinking"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/thinking/provider/codex"
	"github.com/tidwall/gjson"
)

func TestApplyThinkingPassesAdvertisedUltraEffortToCodex(t *testing.T) {
	const clientID = "test-codex-ultra-reasoning"
	const modelID = "test-codex-ultra-model"
	modelRegistry := registry.GetGlobalRegistry()
	modelRegistry.RegisterClient(clientID, "codex", []*registry.ModelInfo{
		{
			ID:       modelID,
			Object:   "model",
			OwnedBy:  "openai",
			Type:     "openai",
			Thinking: &registry.ThinkingSupport{Levels: []string{"low", "medium", "high", "xhigh", "max", "ultra"}},
		},
	})
	t.Cleanup(func() {
		modelRegistry.UnregisterClient(clientID)
	})

	body := []byte(`{"model":"test-codex-ultra-model","reasoning":{"effort":"ultra"}}`)
	result, err := thinking.ApplyThinking(body, modelID, "codex", "codex", "codex")
	if err != nil {
		t.Fatalf("ApplyThinking() error = %v", err)
	}
	if got := gjson.GetBytes(result, "reasoning.effort").String(); got != "ultra" {
		t.Fatalf("reasoning effort = %q, want ultra; body=%s", got, string(result))
	}
}
