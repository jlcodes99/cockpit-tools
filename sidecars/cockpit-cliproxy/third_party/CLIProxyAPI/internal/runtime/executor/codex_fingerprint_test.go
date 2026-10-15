package executor

import (
	"bytes"
	"context"
	"net/http"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

// Persisted/imported legacy modes must not reactivate local rewriting. Exercise
// the real HTTP cache/header path, including lineage, workspace and response IDs.
func TestCodexLegacyFingerprintModesDoNotChangeRequests(t *testing.T) {
	for _, mode := range []string{"", "off", "device", "session", "full", "invalid"} {
		for _, authID := range []string{"auth-a", "auth-b"} {
			for _, session := range []string{"conversation-1", "conversation-2"} {
				payload := []byte(`{"prompt_cache_key":"` + session + `","thread_id":"client-thread","client_metadata":{"x-codex-installation-id":"client-install","x-codex-window-id":"client-thread:0","x-codex-parent-thread-id":"client-parent","thread_id":"client-thread","session_id":"` + session + `","x-codex-turn-metadata":"{\"cwd\":\"/Users/alice/work/repo\",\"remote\":\"https://github.com/acme/repo.git\",\"sha\":\"0123456789abcdef\",\"thread_id\":\"client-thread\",\"turn_id\":\"client-turn\",\"parent_thread_id\":\"client-parent\"}"}}`)
				auth := &cliproxyauth.Auth{ID: authID, Provider: "codex", Metadata: map[string]any{"codex_fingerprint_mode": mode}}
				request := cliproxyexecutor.Request{Model: "gpt-5.5", Payload: payload}
				httpReq, got, state, err := NewCodexExecutor(&config.Config{}).cacheHelper(
					context.Background(), sdktranslator.FormatOpenAIResponse, "http://localhost/responses", auth, request, payload, payload,
				)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(got, payload) {
					t.Fatalf("legacy mode %q changed request: %s", mode, got)
				}
				httpReq.Header.Set("X-Codex-Installation-Id", "client-install")
				httpReq.Header.Set("X-Codex-Parent-Thread-Id", "client-parent")
				httpReq.Header.Set("X-Codex-Turn-Metadata", gjson.GetBytes(payload, "client_metadata.x-codex-turn-metadata").String())
				before := httpReq.Header.Clone()
				applyCodexIdentityConfuseHeaders(httpReq.Header, &state)
				for key, values := range before {
					if httpReq.Header.Get(key) != values[0] {
						t.Fatalf("legacy mode %q rewrote header %s", mode, key)
					}
				}
				if !bytes.Equal(applyCodexIdentityConfuseResponsePayload(payload, state), payload) ||
					!bytes.Equal(applyCodexIdentityExposeResponsePayload(payload, state), payload) {
					t.Fatal("legacy mode rewrote a response")
				}
			}
		}
	}
}

func TestCodexUpstreamIdentityConfuseRemainsExplicit(t *testing.T) {
	auth := &cliproxyauth.Auth{ID: "auth-a", Provider: "codex"}
	payload := []byte(`{"prompt_cache_key":"session-1","client_metadata":{"x-codex-turn-metadata":"{\"turn_id\":\"turn-1\"}"}}`)
	cfg := &config.Config{}
	cfg.Routing.SessionAffinity = true
	for _, enabled := range []bool{false, true} {
		cfg.Codex.IdentityConfuse = enabled
		body, state := applyCodexIdentityConfuseBody(cfg, auth, payload, payload)
		if state.enabled != enabled {
			t.Fatalf("explicit upstream policy ignored: %v", state.enabled)
		}
		if enabled == bytes.Equal(body, payload) {
			t.Fatal("only explicit identity-confuse may rewrite identifiers")
		}
		headers := http.Header{"Session-Id": {"session-1"}}
		applyCodexIdentityConfuseHeaders(headers, &state)
		if !enabled && headers.Get("Session-Id") != "session-1" {
			t.Fatal("default session changed")
		}
	}
}
