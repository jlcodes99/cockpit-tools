package helps

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/codex/collaboration"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

type canonicalToolsHook struct {
	httpPreviousHook
	canonical []byte
}

func (h *canonicalToolsHook) Bind(_ context.Context, r cliproxyexecutor.ContinuityBindRequest) (cliproxyexecutor.ContinuityBindResponse, error) {
	h.bound = r
	return cliproxyexecutor.ContinuityBindResponse{Ticket: r.Ticket, AttemptID: r.AttemptID, Headers: r.Headers, Body: json.RawMessage(h.canonical)}, nil
}

func TestContinuityRebuiltCollaborationToolsKeepPlaintextWirePolicy(t *testing.T) {
	canonical := []byte(`{"model":"test","previous_response_id":"keep-history","input":[{"type":"additional_tools","role":"developer","tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"properties":{"message":{"type":"string","encrypted":true}}}}]}]},{"role":"user","content":"task"}]}`)
	for _, tc := range []struct {
		name                         string
		compat, oauth, enabled, want bool
	}{
		{"compatible API key", true, false, true, true},
		{"optimization off", true, false, false, false},
		{"native API key", false, false, true, false},
		{"OAuth history", true, true, true, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			hook := &canonicalToolsHook{canonical: canonical}
			ctx, _, err := cliproxyexecutor.PrepareContinuity(cliproxyexecutor.WithContinuityHook(context.Background(), hook), cliproxyexecutor.ContinuityPrepareRequest{Body: canonical})
			if err != nil {
				t.Fatal(err)
			}
			auth := &cliproxyauth.Auth{ID: "test-account", Provider: "codex", Attributes: map[string]string{"api_key": "synthetic"}}
			if tc.oauth {
				auth.Attributes = nil
				auth.Metadata = map[string]any{"access_token": "synthetic"}
			}
			headers := http.Header{"User-Agent": []string{"Codex Desktop/0.163.0"}, "Session-Id": []string{"bound-session"}}
			cfg := &config.Config{Codex: config.CodexConfig{OptimizeMultiAgentV2: tc.enabled}}
			projected, _ := OptimizeCodexMultiAgentV2RequestForAuth(ctx, headers, canonical, cfg, auth, tc.compat)
			req, _ := http.NewRequestWithContext(ctx, http.MethodPost, "https://test.invalid/responses", nil)
			req.Header = headers.Clone()
			_, wire, optimized, err := BindCodexMultiAgentV2HTTPRequest(ctx, auth, req, projected, headers, cfg, tc.compat)
			if err != nil {
				t.Fatal(err)
			}
			if optimized != tc.want {
				t.Fatalf("optimized=%v want %v; %s", optimized, tc.want, wire)
			}
			if got := gjson.GetBytes(wire, "input.0.tools.0.tools.0.parameters.properties.message.encrypted").Exists(); got == tc.want {
				t.Fatalf("wire encryption flag=%v; %s", got, wire)
			}
			if gjson.GetBytes(wire, "previous_response_id").String() != "keep-history" || req.Header.Get("Session-Id") != "bound-session" {
				t.Fatal("continuation identity changed")
			}
			if req.ContentLength != int64(len(wire)) {
				t.Fatal("stale content length")
			}
			actual, _ := io.ReadAll(req.Body)
			clone, _ := req.GetBody()
			replay, _ := io.ReadAll(clone)
			clone.Close()
			if string(actual) != string(wire) || string(replay) != string(wire) {
				t.Fatal("HTTP bytes disagree with returned policy body")
			}
			response := []byte(`{"type":"response.output_item.done","item":{"type":"function_call","name":"spawn_agent","namespace":"collaboration","arguments":"{\"message\":\"plain task\"}"}}`)
			response = MarkCodexMultiAgentV2PlaintextResponse(ctx, headers, response, collaboration.PlaintextPolicyForRequest(wire, optimized), cfg)
			if marked := gjson.GetBytes(response, "item.encrypted_function_args").Raw == "[]"; marked != tc.want {
				t.Fatalf("plaintext marker=%v; %s", marked, response)
			}
		})
	}
}
