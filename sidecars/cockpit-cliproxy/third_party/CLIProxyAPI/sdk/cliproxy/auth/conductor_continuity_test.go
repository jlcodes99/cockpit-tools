package auth

import (
	"bytes"
	"context"
	"errors"
	"net/http"
	"testing"

	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/codex/claude"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/codex/gemini"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/codex/openai/chat-completions"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/codex/openai/responses"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

type managerContinuityHook struct {
	selfContained bool
	noPreferred   bool
	prepareErr    error
	prepared      []cliproxyexecutor.ContinuityPrepareRequest
}

func (h *managerContinuityHook) Prepare(_ context.Context, r cliproxyexecutor.ContinuityPrepareRequest) (cliproxyexecutor.ContinuityPrepareResponse, error) {
	h.prepared = append(h.prepared, r)
	if h.prepareErr != nil {
		return cliproxyexecutor.ContinuityPrepareResponse{}, h.prepareErr
	}
	preferred := "continuity-original"
	if h.noPreferred {
		preferred = ""
	}
	return cliproxyexecutor.ContinuityPrepareResponse{Ticket: "continuity-test-ticket", PreferredAccountID: preferred, SelfContained: h.selfContained}, nil
}
func (h *managerContinuityHook) Bind(context.Context, cliproxyexecutor.ContinuityBindRequest) (cliproxyexecutor.ContinuityBindResponse, error) {
	panic("unused by selection-only executor")
}
func (h *managerContinuityHook) Observe(context.Context, cliproxyexecutor.ContinuityObserveRequest) (cliproxyexecutor.ContinuityObserveResponse, error) {
	panic("unused by selection-only executor")
}
func (h *managerContinuityHook) Finish(context.Context, cliproxyexecutor.ContinuityFinishRequest) error {
	return nil
}

func TestContinuityPrepareNormalizesOnlyCanonicalCodexBoundary(t *testing.T) {
	for _, tc := range []struct {
		format sdktranslator.Format
		body   string
	}{
		{sdktranslator.FormatOpenAIResponse, `{"input":[{"role":"user","content":"hello"}]}`},
		{sdktranslator.FormatOpenAIResponse, `{"input":"hello"}`},
		{sdktranslator.FormatOpenAI, `{"messages":[{"role":"user","content":"hello"}]}`},
		{sdktranslator.FormatClaude, `{"max_tokens":128,"messages":[{"role":"user","content":"hello"}]}`},
		{sdktranslator.FormatGemini, `{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}`},
	} {
		t.Run(tc.format.String(), func(t *testing.T) {
			hook := &managerContinuityHook{selfContained: true}
			ctx := cliproxyexecutor.WithContinuityHook(context.Background(), hook)
			req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: []byte(tc.body), Format: tc.format}
			opts := cliproxyexecutor.Options{SourceFormat: tc.format, OriginalRequest: []byte(tc.body)}
			preparedCtx, out, err := prepareExecutionContinuity(ctx, []string{"codex"}, req, opts)
			if err != nil {
				t.Fatal(err)
			}
			if !cliproxyexecutor.ContinuityActive(preparedCtx) || len(hook.prepared) != 1 || !gjson.GetBytes(hook.prepared[0].Body, "input").IsArray() {
				t.Fatalf("not canonical Responses: %+v", hook.prepared)
			}
			if out.SourceFormat != tc.format || !bytes.Equal(out.OriginalRequest, []byte(tc.body)) || !bytes.Equal(req.Payload, []byte(tc.body)) {
				t.Fatal("client protocol was mutated")
			}
			for _, providers := range [][]string{{"claude"}, {"gemini"}, {"xai"}} {
				if _, _, err := prepareExecutionContinuity(ctx, providers, req, opts); err != nil {
					t.Fatal(err)
				}
			}
			if len(hook.prepared) != 1 {
				t.Fatal("native protocol claimed a Responses transaction")
			}
		})
	}
}

func TestContinuityMixedProvidersFenceOnlyCodexAfterPrepareFailure(t *testing.T) {
	for _, tc := range []struct {
		name, body                       string
		noPreferred, fail, wantCodexOnly bool
	}{
		{"new-plaintext", `{"input":"hello"}`, true, false, false},
		{"known-owner", `{"input":[]}`, false, false, true},
		{"removed-owner-opaque", `{"input":[{"type":"reasoning","encrypted_content":"synthetic-cipher"}]}`, true, false, true},
		{"previous-response", `{"previous_response_id":"resp","input":[]}`, true, false, true},
		{"foreign-native-history", `{"input":[]}`, true, true, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			hook := &managerContinuityHook{selfContained: true, noPreferred: tc.noPreferred}
			if tc.fail {
				hook.prepareErr = errors.New("synthetic foreign native ciphertext")
			}
			req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: []byte(tc.body)}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, OriginalRequest: req.Payload}
			ctx, out, err := prepareExecutionContinuity(cliproxyexecutor.WithContinuityHook(context.Background(), hook), []string{"codex", "claude"}, req, opts)
			if err != nil {
				t.Fatal(err)
			}
			providers := continuityExecutionProviders(ctx, []string{"codex", "claude"}, out)
			if tc.fail {
				if len(providers) != 1 || providers[0] != "claude" || cliproxyexecutor.ContinuityPreparationError(ctx) == nil || cliproxyexecutor.ContinuityActive(ctx) {
					t.Fatalf("native fallback/fence missing: %v", providers)
				}
			} else if tc.wantCodexOnly {
				if len(providers) != 1 || providers[0] != "codex" {
					t.Fatalf("Codex history escaped provider pool: %v", providers)
				}
			} else if len(providers) != 2 {
				t.Fatalf("fresh plaintext routing changed: %v", providers)
			}
			if !bytes.Equal(out.OriginalRequest, req.Payload) || out.SourceFormat != opts.SourceFormat {
				t.Fatal("native payload/protocol mutated")
			}
		})
	}
}

func TestContinuityMixedProvidersPreserveOwnershipAndHistoryErrors(t *testing.T) {
	for _, code := range []string{"upstream_state_caller_mismatch", "upstream_state_owner_changed", "upstream_state_owner_conflict", "upstream_turn_state_mismatch", "mixed_upstream_state", "mixed_upstream_generation", "account_binding_unavailable", "continuity_account_scope", "continuation_history_incomplete", "previous_response_not_found"} {
		t.Run(code, func(t *testing.T) {
			want := errors.New(`{"error":{"code":"` + code + `","message":"protected history"}}`)
			hook := &managerContinuityHook{prepareErr: want}
			body := []byte(`{"input":[{"type":"reasoning","encrypted_content":"other-caller-cipher"}]}`)
			req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: body}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, OriginalRequest: body}
			ctx, out, err := prepareExecutionContinuity(cliproxyexecutor.WithContinuityHook(context.Background(), hook), []string{"codex", "xai"}, req, opts)
			if err != want || cliproxyexecutor.ContinuityActive(ctx) || cliproxyexecutor.ContinuityPreparationError(ctx) != nil {
				t.Fatalf("protected history fell through to another provider: %v", err)
			}
			if !bytes.Equal(out.OriginalRequest, body) || !bytes.Equal(req.Payload, body) {
				t.Fatal("rejected history was changed")
			}
		})
	}
}

type managerContinuityExecutor struct {
	*replaceAwareExecutor
	selected   []string
	refreshes  int
	failStatus int
}

func (e *managerContinuityExecutor) Execute(_ context.Context, a *Auth, _ cliproxyexecutor.Request, _ cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	e.selected = append(e.selected, a.ID)
	if a.ID == "continuity-original" {
		return cliproxyexecutor.Response{}, &Error{HTTPStatus: e.failStatus, Message: "synthetic upstream failure"}
	}
	return cliproxyexecutor.Response{Payload: []byte(`{}`)}, nil
}
func (e *managerContinuityExecutor) ExecuteStream(ctx context.Context, a *Auth, r cliproxyexecutor.Request, o cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	_, err := e.Execute(ctx, a, r, o)
	if err != nil {
		return nil, err
	}
	ch := make(chan cliproxyexecutor.StreamChunk, 1)
	ch <- cliproxyexecutor.StreamChunk{Payload: []byte(`data: {}`)}
	close(ch)
	return &cliproxyexecutor.StreamResult{Headers: http.Header{}, Chunks: ch}, nil
}
func (e *managerContinuityExecutor) Refresh(_ context.Context, a *Auth) (*Auth, error) {
	e.refreshes++
	return a, nil
}

func TestContinuityAutomaticFailoverPreservesStateDependentBinding(t *testing.T) {
	for _, stream := range []bool{false, true} {
		for _, tc := range []struct {
			name          string
			selfContained bool
			status, want  int
		}{
			{"opaque-incremental-pinned", false, 429, 1},
			{"complete-context-quota-failover", true, 429, 2},
			{"complete-context-server-failover", true, 503, 2},
		} {
			t.Run(tc.name+map[bool]string{false: "-nonstream", true: "-stream"}[stream], func(t *testing.T) {
				hook := &managerContinuityHook{selfContained: tc.selfContained}
				manager := NewManager(nil, nil, nil)
				manager.SetRetryConfig(0, 0, 3)
				exec := &managerContinuityExecutor{replaceAwareExecutor: &replaceAwareExecutor{id: "codex"}, failStatus: tc.status}
				manager.RegisterExecutor(exec)
				registerRetryRoundLocalAuths(t, manager, "codex", "continuity-test-model", map[string]int{"continuity-original": 0, "continuity-backup": 0})
				ctx := cliproxyexecutor.WithContinuityHook(context.Background(), hook)
				req := cliproxyexecutor.Request{Model: "continuity-test-model", Payload: []byte(`{"input":[]}`)}
				opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: stream}
				if stream {
					result, _ := manager.ExecuteStream(ctx, []string{"codex"}, req, opts)
					if result != nil {
						for range result.Chunks {
						}
					}
				} else {
					_, _ = manager.Execute(ctx, []string{"codex"}, req, opts)
				}
				if len(exec.selected) != tc.want || exec.selected[0] != "continuity-original" || exec.refreshes != 0 {
					t.Fatalf("selected=%v refreshes=%d, want %d sends", exec.selected, exec.refreshes, tc.want)
				}
			})
		}
	}
}
