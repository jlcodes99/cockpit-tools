package handlers

import (
	"context"
	"sync/atomic"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdkconfig "github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

type tierBootstrapExecutor struct {
	failOnceStreamExecutor
	current *atomic.Value
	tiers   []string
}

func (e *tierBootstrapExecutor) ExecuteStream(ctx context.Context, auth *coreauth.Auth, req coreexecutor.Request, opts coreexecutor.Options) (*coreexecutor.StreamResult, error) {
	tier, _ := coreexecutor.ServiceTierDefaultSnapshot(ctx)
	e.tiers = append(e.tiers, tier)
	e.current.Store("")
	return e.failOnceStreamExecutor.ExecuteStream(ctx, auth, req, opts)
}

func TestServiceTierDefaultSurvivesHandlerBootstrapRetry(t *testing.T) {
	var current atomic.Value
	current.Store("priority")
	exec := &tierBootstrapExecutor{current: &current}
	manager := coreauth.NewManager(nil, nil, nil)
	manager.RegisterExecutor(exec)
	const model = "tier-bootstrap-model"
	for _, id := range []string{"tier-bootstrap-1", "tier-bootstrap-2"} {
		auth := &coreauth.Auth{ID: id, Provider: "codex", Status: coreauth.StatusActive}
		if _, err := manager.Register(context.Background(), auth); err != nil {
			t.Fatal(err)
		}
		registry.GetGlobalRegistry().RegisterClient(id, "codex", []*registry.ModelInfo{{ID: model}})
		t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(id) })
	}
	handler := NewBaseAPIHandlers(&sdkconfig.SDKConfig{Streaming: sdkconfig.StreamingConfig{BootstrapRetries: 1}}, manager)
	ctx := coreexecutor.WithServiceTierDefaultProvider(context.Background(), func() string { return current.Load().(string) })
	data, _, errs := handler.ExecuteStreamWithAuthManager(ctx, "openai", model, []byte(`{"model":"tier-bootstrap-model"}`), "")
	for range data {
	}
	for err := range errs {
		if err != nil {
			t.Fatalf("execution failed: %v", err)
		}
	}
	if len(exec.tiers) != 2 || exec.tiers[0] != "priority" || exec.tiers[1] != "priority" {
		t.Fatalf("retry tiers = %#v", exec.tiers)
	}
	data, _, errs = handler.ExecuteStreamWithAuthManager(ctx, "openai", model, []byte(`{"model":"tier-bootstrap-model"}`), "")
	for range data {
	}
	for err := range errs {
		if err != nil {
			t.Fatalf("next execution failed: %v", err)
		}
	}
	if len(exec.tiers) != 3 || exec.tiers[2] != "" {
		t.Fatalf("next request tiers = %#v", exec.tiers)
	}
}
