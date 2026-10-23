package auth

import (
	"context"
	"errors"
	"net/http"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestCanceledBootstrapObserverKeepsAttemptSnapshot(t *testing.T) {
	hook := &canceledDrainHook{observed: make(chan Result, 4)}
	manager := NewManager(nil, nil, hook)
	original, errRegister := manager.Register(t.Context(), &Auth{ID: "bootstrap-snapshot", Provider: "codex", Metadata: map[string]any{"access_token": "old"}})
	if errRegister != nil {
		t.Fatal(errRegister)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	source := make(chan cliproxyexecutor.StreamChunk)
	opts := cliproxyexecutor.Options{Metadata: map[string]any{"caller_scope": "original"}, Headers: http.Header{"X-Test": {"original"}}, OriginalRequest: []byte("original")}
	manager.observeCanceledBootstrapStream(ctx, original, "codex", "test", "test", opts, ctx.Err(), &cliproxyexecutor.StreamResult{Chunks: source}, false)
	// Change both caller-owned data and the auth after the observer was handed off.
	opts.Metadata["caller_scope"] = "changed"
	opts.Headers.Set("X-Test", "changed")
	opts.OriginalRequest[0] = 'x'
	replacement := original.Clone()
	replacement.Metadata["access_token"] = "new"
	if _, errUpdate := manager.Update(t.Context(), replacement); errUpdate != nil {
		t.Fatal(errUpdate)
	}
	source <- cliproxyexecutor.StreamChunk{Err: issue6503QuotaError{retryAfter: time.Hour}}
	close(source)
	result := awaitCanceledDrainResult(t, hook)
	if !result.StaleCredential || result.Success || result.Options.Metadata["caller_scope"] != "original" || result.Options.Headers.Get("X-Test") != "original" || string(result.Options.OriginalRequest) != "original" || result.CredentialVersion != original.CredentialVersion {
		t.Fatalf("attempt snapshot was not retained: %#v", result)
	}
	latest, _ := manager.GetByID(original.ID)
	if latest.Unavailable || latest.Failed != 0 || latest.Success != 0 {
		t.Fatalf("old attempt changed replacement: %#v", latest)
	}
}

func TestCanceledBootstrapObserverIgnoresPureCancellation(t *testing.T) {
	hook := &canceledDrainHook{observed: make(chan Result, 4)}
	manager := NewManager(nil, nil, hook)
	credential, errRegister := manager.Register(t.Context(), &Auth{ID: "bootstrap-pure-cancel", Provider: "codex"})
	if errRegister != nil {
		t.Fatal(errRegister)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	source := make(chan cliproxyexecutor.StreamChunk)
	manager.observeCanceledBootstrapStream(ctx, credential, "codex", "test", "test", cliproxyexecutor.Options{}, ctx.Err(), &cliproxyexecutor.StreamResult{Chunks: source}, false)
	for _, chunk := range []cliproxyexecutor.StreamChunk{{Payload: []byte("normal output")}, {Err: context.Canceled}, {}} {
		source <- chunk
	}
	close(source)
	latest, _ := manager.GetByID(credential.ID)
	if latest.Unavailable || latest.Failed != 0 || latest.Success != 0 || len(hook.Results()) != 0 {
		t.Fatalf("pure cancellation changed auth: %#v", latest)
	}
}

type canceledRetryBootstrapExecutor struct {
	issue6503StreamExecutor
	calls  int
	source chan cliproxyexecutor.StreamChunk
}

func (e *canceledRetryBootstrapExecutor) ExecuteStream(context.Context, *Auth, cliproxyexecutor.Request, cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	e.calls++
	if e.calls == 1 {
		return streamErrorResult(nil, &Error{HTTPStatus: http.StatusUnauthorized, Message: "token invalidated"}), nil
	}
	e.cancel()
	return &cliproxyexecutor.StreamResult{Chunks: e.source}, nil
}

func TestManagerObservesQuotaAfterCanceledRefreshBootstrap(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	hook := &canceledDrainHook{observed: make(chan Result, 4)}
	manager := NewManager(nil, nil, hook)
	manager.SetRetryConfig(0, 0, 0)
	exec := &canceledRetryBootstrapExecutor{issue6503StreamExecutor: issue6503StreamExecutor{cancel: cancel}, source: make(chan cliproxyexecutor.StreamChunk)}
	manager.RegisterExecutor(exec)
	credential := &Auth{ID: uuid.NewString(), Provider: exec.Identifier(), Metadata: map[string]any{"access_token": "old", "refresh_token": "refresh"}}
	model := uuid.NewString()
	modelRegistry := registry.GetGlobalRegistry()
	modelRegistry.RegisterClient(credential.ID, credential.Provider, []*registry.ModelInfo{{ID: model}})
	defer modelRegistry.UnregisterClient(credential.ID)
	if _, errRegister := manager.Register(t.Context(), credential); errRegister != nil {
		t.Fatal(errRegister)
	}
	_, errStream := manager.ExecuteStream(ctx, []string{credential.Provider}, cliproxyexecutor.Request{Model: model}, cliproxyexecutor.Options{})
	if !errors.Is(errStream, context.Canceled) || exec.calls != 2 {
		t.Fatalf("refresh bootstrap did not cancel: calls=%d error=%v", exec.calls, errStream)
	}
	exec.source <- cliproxyexecutor.StreamChunk{Err: issue6503QuotaError{retryAfter: time.Hour}}
	close(exec.source)
	result := awaitCanceledDrainResult(t, hook)
	if result.Success || result.RetryAfter == nil || !result.CredentialScope {
		t.Fatalf("refresh bootstrap recorded cancellation instead of quota: %#v", result)
	}
	latest, _ := manager.GetByID(credential.ID)
	if !latest.Unavailable || latest.Failed != 1 || latest.Success != 0 {
		t.Fatalf("refresh bootstrap quota missing: %#v", latest)
	}
}
