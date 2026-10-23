package multiagentv2

import (
	"context"
	"net/http"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/codex/collaboration"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
)

// MarkCodexMultiAgentV2PlaintextResponse is independent of namespace restoration.
// Only an eligible request's final transmitted declarations establish plaintext.
func MarkCodexMultiAgentV2PlaintextResponse(ctx context.Context, headers http.Header, payload []byte, policy collaboration.PlaintextPolicy, cfg *config.Config) []byte {
	if !codexMultiAgentV2Enabled(ctx, headers, cfg) {
		return payload
	}
	return policy.MarkCalls(payload)
}

// MarkCodexMultiAgentV2PlaintextWireEvent adds only plaintext metadata before
// durable observation, retaining upstream aliases and every original field.
func MarkCodexMultiAgentV2PlaintextWireEvent(ctx context.Context, headers http.Header, payload []byte, policy collaboration.PlaintextPolicy, cfg *config.Config) []byte {
	if !codexMultiAgentV2Enabled(ctx, headers, cfg) {
		return payload
	}
	return policy.MarkWireCalls(payload)
}
