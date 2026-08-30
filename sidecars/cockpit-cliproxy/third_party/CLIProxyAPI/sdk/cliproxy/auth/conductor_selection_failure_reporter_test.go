package auth

import (
	"context"
	"strings"
	"testing"
	"time"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

type selectionFailureReportingSelector struct {
	Selector
	called     bool
	candidates []*Auth
}

func (s *selectionFailureReportingSelector) ReportAuthSelectionFailure(_ context.Context, _ string, _ string, candidates []*Auth, _ error) error {
	s.called = true
	s.candidates = candidates
	return &Error{Code: "auth_unavailable", Message: "pool diagnostics attached", HTTPStatus: 503}
}

func TestManagerReportsAvailabilityFailureAndUsesReportedError(t *testing.T) {
	const model = "gpt-5.5"
	selector := &selectionFailureReportingSelector{Selector: &RoundRobinSelector{}}
	manager := NewManager(nil, selector, nil)
	manager.RegisterExecutor(schedulerTestExecutor{provider: "codex"})
	_, err := manager.Register(context.Background(), &Auth{
		ID:             "codex-auth",
		Provider:       "codex",
		Status:         StatusActive,
		Unavailable:    true,
		NextRetryAfter: time.Now().Add(time.Hour),
	})
	if err != nil {
		t.Fatalf("Register() error = %v", err)
	}
	registerSchedulerModels(t, "codex", model, "codex-auth")

	_, err = manager.Execute(context.Background(), []string{"codex"}, cliproxyexecutor.Request{Model: model}, cliproxyexecutor.Options{})
	if err == nil || !strings.Contains(err.Error(), "pool diagnostics attached") {
		t.Fatalf("Execute() error = %v, want reporter error", err)
	}
	if !selector.called {
		t.Fatal("availability failure reporter was not called")
	}
	if len(selector.candidates) != 1 || selector.candidates[0].ID != "codex-auth" {
		t.Fatalf("reporter candidates = %#v, want registered auth", selector.candidates)
	}
}
