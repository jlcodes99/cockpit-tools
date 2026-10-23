package auth

import (
	"context"
	"net/http"
	"strings"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

type unsentPoolExecutor struct {
	prepareID string
	sentID    string
	calls     []string
	stop      bool
	bootstrap bool
}

func (*unsentPoolExecutor) Identifier() string { return "codex" }
func (e *unsentPoolExecutor) ShouldPrepareRequestAuth(auth *Auth) bool {
	return auth.ID == e.prepareID
}
func (*unsentPoolExecutor) PrepareRequestAuth(context.Context, *Auth) (*Auth, error) {
	return nil, &Error{HTTPStatus: http.StatusForbidden, Message: "local preparation unavailable"}
}
func (e *unsentPoolExecutor) execute(ctx context.Context, auth *Auth) error {
	e.calls = append(e.calls, auth.ID)
	if err := cliproxyexecutor.CheckUpstreamSend(ctx, auth.ID); err != nil {
		return err
	}
	cliproxyexecutor.MarkUpstreamAttempt(ctx)
	if auth.ID == e.sentID {
		err := &Error{Code: "usage_limit_reached", HTTPStatus: http.StatusTooManyRequests, Message: "real upstream quota rejection"}
		if e.stop {
			return wrapRequestStopError(err)
		}
		return err
	}
	return nil
}
func (e *unsentPoolExecutor) Execute(ctx context.Context, auth *Auth, _ cliproxyexecutor.Request, _ cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	err := e.execute(ctx, auth)
	return cliproxyexecutor.Response{Payload: []byte("ok")}, err
}
func (e *unsentPoolExecutor) CountTokens(ctx context.Context, auth *Auth, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	return e.Execute(ctx, auth, req, opts)
}
func (e *unsentPoolExecutor) ExecuteStream(ctx context.Context, auth *Auth, _ cliproxyexecutor.Request, _ cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	if err := e.execute(ctx, auth); err != nil {
		if e.bootstrap {
			chunks := make(chan cliproxyexecutor.StreamChunk, 1)
			chunks <- cliproxyexecutor.StreamChunk{Err: err}
			close(chunks)
			return &cliproxyexecutor.StreamResult{Chunks: chunks}, nil
		}
		return nil, err
	}
	return successStreamResult(), nil
}

func TestExecutionStopPreservesQuotaCooldownWithoutReplayingAnotherAccount(t *testing.T) {
	for _, path := range []string{"execute", "count", "stream", "stream-bootstrap"} {
		t.Run(path, func(t *testing.T) {
			executor := &unsentPoolExecutor{sentID: "00-output-started", stop: true, bootstrap: path == "stream-bootstrap"}
			hook := &resultCaptureHook{}
			m := NewManager(nil, quotaAttemptIsolationSelector{}, hook)
			m.RegisterExecutor(executor)
			m.SetRetryConfig(2, 0, 2)
			model := "business-output-stop-" + path
			for _, id := range []string{executor.sentID, "01-backup"} {
				if _, err := m.Register(context.Background(), &Auth{ID: id, Provider: "codex", Status: StatusActive}); err != nil {
					t.Fatal(err)
				}
				registry.GetGlobalRegistry().RegisterClient(id, "codex", []*registry.ModelInfo{{ID: model}})
				t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(id) })
			}
			err := invokeUnsentPoolPath(context.Background(), m, model, path)
			if statusCodeFromError(err) != 429 || !strings.Contains(err.Error(), "real upstream quota rejection") {
				t.Fatalf("lost real quota rejection: %v", err)
			}
			if len(executor.calls) != 1 || executor.calls[0] != executor.sentID {
				t.Fatalf("unsafe account replay after execution started: %v", executor.calls)
			}
			auth, _ := m.GetByID(executor.sentID)
			if auth.Failed != 1 || len(auth.ModelStates) == 0 {
				t.Fatalf("execution stop lost real rejection health: %#v", auth)
			}
			if results := hook.Results(); len(results) != 1 || results[0].Success || results[0].Error.HTTPStatus != 429 {
				t.Fatalf("quota result = %#v, want one real rejection", results)
			}
		})
	}
}
func (*unsentPoolExecutor) Refresh(_ context.Context, auth *Auth) (*Auth, error) {
	return auth, nil
}
func (*unsentPoolExecutor) HttpRequest(context.Context, *Auth, *http.Request) (*http.Response, error) {
	return nil, nil
}

func invokeUnsentPoolPath(ctx context.Context, m *Manager, model, path string) error {
	req := cliproxyexecutor.Request{Model: model}
	switch path {
	case "execute":
		_, err := m.Execute(ctx, []string{"codex"}, req, cliproxyexecutor.Options{})
		return err
	case "count":
		_, err := m.ExecuteCount(ctx, []string{"codex"}, req, cliproxyexecutor.Options{})
		return err
	default:
		stream, err := m.ExecuteStream(ctx, []string{"codex"}, req, cliproxyexecutor.Options{Stream: true})
		if err == nil {
			for range stream.Chunks {
			}
		}
		return err
	}
}

func TestUnsentCandidatesDoNotConsumeCredentialBudgetOrRecordFailures(t *testing.T) {
	for _, path := range []string{"execute", "count", "stream"} {
		t.Run(path, func(t *testing.T) {
			executor := &unsentPoolExecutor{prepareID: "00-prepare", sentID: "02-upstream"}
			hook := &resultCaptureHook{}
			m := NewManager(nil, quotaAttemptIsolationSelector{}, hook)
			m.RegisterExecutor(executor)
			m.SetRetryConfig(0, 0, 2)
			model := "unsent-budget-" + path
			ids := []string{executor.prepareID, "01-guard", executor.sentID, "03-backup"}
			for _, id := range ids {
				_, err := m.Register(context.Background(), &Auth{ID: id, Provider: "codex", Status: StatusActive})
				if err != nil {
					t.Fatal(err)
				}
				registry.GetGlobalRegistry().RegisterClient(id, "codex", []*registry.ModelInfo{{ID: model}})
				t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(id) })
			}
			ctx := cliproxyexecutor.WithUpstreamSendGuard(context.Background(), func(_ context.Context, id string) error {
				if id == "01-guard" {
					return &Error{Code: "quota_reserved", HTTPStatus: http.StatusTooManyRequests, Message: "local admission changed"}
				}
				return nil
			})
			if err := invokeUnsentPoolPath(ctx, m, model, path); err != nil {
				t.Fatalf("local candidates consumed the two real-send budget: %v", err)
			}
			for _, id := range ids[:2] {
				auth, _ := m.GetByID(id)
				if auth.Failed != 0 || auth.Success != 0 || auth.Unavailable || auth.LastError != nil || len(auth.ModelStates) != 0 {
					t.Fatalf("unsent candidate recorded an anomaly: %#v", auth)
				}
			}
			results := hook.Results()
			if len(results) != 2 || results[0].AuthID != executor.sentID || results[0].Success || !results[1].Success || results[1].AuthID != "03-backup" {
				t.Fatalf("execution results = %#v, want real 429 then backup success", results)
			}
			upstream, _ := m.GetByID(executor.sentID)
			if upstream.Failed != 1 || len(upstream.ModelStates) == 0 {
				t.Fatalf("real upstream rejection was not recorded: %#v", upstream)
			}
		})
	}
}

func TestRealUpstreamErrorOutranksLaterUnsentCandidates(t *testing.T) {
	for _, path := range []string{"execute", "count", "stream"} {
		t.Run(path, func(t *testing.T) {
			executor := &unsentPoolExecutor{prepareID: "02-prepare", sentID: "00-upstream"}
			hook := &resultCaptureHook{}
			m := NewManager(nil, quotaAttemptIsolationSelector{}, hook)
			m.RegisterExecutor(executor)
			m.SetRetryConfig(0, 0, 2)
			model := "unsent-error-priority-" + path
			for _, id := range []string{executor.sentID, "01-guard", executor.prepareID} {
				if _, err := m.Register(context.Background(), &Auth{ID: id, Provider: "codex", Status: StatusActive}); err != nil {
					t.Fatal(err)
				}
				registry.GetGlobalRegistry().RegisterClient(id, "codex", []*registry.ModelInfo{{ID: model}})
				t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(id) })
			}
			ctx := cliproxyexecutor.WithUpstreamSendGuard(context.Background(), func(_ context.Context, id string) error {
				if id == "01-guard" {
					return &Error{Code: "quota_reserved", HTTPStatus: 429, Message: "not sent"}
				}
				return nil
			})
			err := invokeUnsentPoolPath(ctx, m, model, path)
			if statusCodeFromError(err) != 429 || !strings.Contains(err.Error(), "real upstream quota rejection") {
				t.Fatalf("real upstream error replaced by unsent candidate: %v", err)
			}
			if results := hook.Results(); len(results) != 1 || results[0].AuthID != executor.sentID {
				t.Fatalf("unsent candidates emitted result hooks: %#v", results)
			}
		})
	}
}
