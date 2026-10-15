package executor

import (
	"net/http"
	"testing"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestApplyCodexRoutingHint(t *testing.T) {
	h := make(http.Header)
	applyCodexRoutingHint(h, &cliproxyauth.Auth{Provider: "codex"}, "gpt-5-codex", []byte(`{"service_tier":"priority"}`))
	if got := h.Get("X-Codex-Routing-Hint"); got != "model=gpt-5-codex;tier=priority" {
		t.Fatalf("routing hint = %q", got)
	}
	apiKey := &cliproxyauth.Auth{Provider: "codex", Attributes: map[string]string{"api_key": "secret"}}
	h = make(http.Header)
	applyCodexRoutingHint(h, apiKey, "gpt-5-codex", []byte(`{"service_tier":"priority"}`))
	if got := h.Get("X-Codex-Routing-Hint"); got != "" {
		t.Fatalf("API-key request received routing hint %q", got)
	}
}
