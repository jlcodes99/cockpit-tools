package multiagentv2

import (
	"context"
	"net/http"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/codex/collaboration"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/tidwall/gjson"
)

func TestPlaintextResponseEligibilityIsIndependentOfNamespaceOptimization(t *testing.T) {
	request := []byte(`{"tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"send_message","parameters":{"properties":{"message":{"type":"string","encrypted":true}}}}]}]}`)
	payload := []byte(`{"type":"function_call","namespace":"collaboration","name":"send_message","arguments":"{}"}`)
	for _, tc := range []struct {
		name, ua      string
		enabled, want bool
	}{
		{"official enabled", "codex-tui/0.145.0", true, true},
		{"official disabled", "codex-tui/0.145.0", false, false},
		{"other client", "curl/8", true, false},
	} {
		headers := http.Header{"User-Agent": {tc.ua}}
		cfg := &config.Config{Codex: config.CodexConfig{OptimizeMultiAgentV2: tc.enabled}}
		final, namespaceOptimized := OptimizeCodexMultiAgentV2Request(context.Background(), headers, request, cfg)
		if namespaceOptimized {
			t.Fatal("send_message-only request unexpectedly changed namespace bool")
		}
		got := MarkCodexMultiAgentV2PlaintextResponse(context.Background(), headers, payload, collaboration.PlaintextPolicyForRequest(final), cfg)
		if marked := gjson.GetBytes(got, "encrypted_function_args").Raw == "[]"; marked != tc.want {
			t.Fatalf("%s = %s", tc.name, got)
		}
	}
	if got := RestoreCodexMultiAgentV2Response(payload, true); string(got) != string(payload) {
		t.Fatal("namespace restore inferred plaintext")
	}
}
