package thinking_test

import (
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/thinking"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/thinking/provider/claude"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/thinking/provider/kimi"
	"github.com/tidwall/gjson"
)

// Claude Code adaptive effort is converted to the K2.5 token-budget contract.
// KimiExecutor delegates to ClaudeExecutor, so ApplyThinking sees claude/claude.
func TestKimiClaudeMessagesMaxUsesTokenBudget(t *testing.T) {
	models := registry.GetKimiModels()
	reg := registry.GetGlobalRegistry()
	clientID := "test-kimi-max-clamp"
	reg.RegisterClient(clientID, "kimi", models)
	t.Cleanup(func() { reg.UnregisterClient(clientID) })

	body := []byte(`{"model":"kimi-k2.5","messages":[{"role":"user","content":"hi"}],"thinking":{"type":"adaptive"},"output_config":{"effort":"max"}}`)
	out, err := thinking.ApplyThinking(body, "kimi-k2.5", "claude", "claude", "claude")
	if err != nil {
		t.Fatalf("ApplyThinking returned error: %v", err)
	}
	if got := gjson.GetBytes(out, "thinking.type").String(); got != "enabled" {
		t.Fatalf("thinking.type = %q, want enabled", got)
	}
	if got := gjson.GetBytes(out, "thinking.budget_tokens").Int(); got != 32000 {
		t.Fatalf("thinking.budget_tokens = %d, want 32000", got)
	}
	if gjson.GetBytes(out, "output_config.effort").Exists() {
		t.Fatalf("budget-based K2.5 must not receive adaptive effort: %s", out)
	}
}
