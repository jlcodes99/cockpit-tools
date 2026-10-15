package main

import (
	"context"
	"net/http"
	"testing"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

// The manager only checks the outermost selector for OnResult support. Every
// cockpit selector wrapper must forward it until session affinity receives it;
// otherwise a failed bound account is never unbound and the next request keeps
// reusing the exhausted credential.
func TestCoreAuthSelectorForwardsSessionAffinityResults(t *testing.T) {
	headers := make(http.Header)
	headers.Set("X-Session-ID", "session-failover")
	opts := cliproxyexecutor.Options{Headers: headers}

	authA := &coreauth.Auth{ID: "a.json", Provider: "codex", Status: coreauth.StatusActive}
	authB := &coreauth.Auth{ID: "b.json", Provider: "codex", Status: coreauth.StatusActive}
	accountA := &accountSpec{
		ID:       "account-a",
		Email:    "a@example.com",
		AuthID:   "a.json",
		AuthKind: "oauth",
	}
	accountB := &accountSpec{
		ID:       "account-b",
		Email:    "b@example.com",
		AuthID:   "b.json",
		AuthKind: "oauth",
	}
	m := &manifest{
		Accounts: []accountSpec{*accountA, *accountB},
		ModelIDs: []string{"gpt-5.4"},
		accountByID: map[string]*accountSpec{
			"account-a": accountA,
			"account-b": accountB,
		},
		accountByAuthID: map[string]*accountSpec{
			"a.json": accountA,
			"b.json": accountB,
		},
		accountByAPIKey: map[string]*accountSpec{},
		originalIndexByID: map[string]int{
			"account-a": 0,
			"account-b": 1,
		},
	}
	cfg := &config.Config{}
	cfg.Routing.SessionAffinity = true
	cfg.Routing.SessionAffinityTTL = time.Minute.String()

	selector := buildCoreAuthSelectorWithConcurrency(
		cfg,
		&cockpitSelector{manifest: m},
		m,
		nil,
		newRequestUsageTracker(),
	)
	if stoppable, ok := selector.(coreauth.StoppableSelector); ok {
		defer stoppable.Stop()
	}

	first, err := selector.Pick(
		context.Background(),
		"codex",
		"gpt-5.4",
		opts,
		[]*coreauth.Auth{authA, authB},
	)
	if err != nil {
		t.Fatalf("first Pick: %v", err)
	}
	if first == nil || first.ID != "a.json" {
		t.Fatalf("expected first session binding on a.json, got %#v", first)
	}

	listener, ok := selector.(interface{ OnResult(coreauth.Result) })
	if !ok {
		t.Fatal("outermost selector must forward session-affinity result notifications")
	}
	listener.OnResult(coreauth.Result{
		AuthID:   first.ID,
		Provider: "codex",
		Model:    "gpt-5.4",
		Success:  false,
		Error: &coreauth.Error{
			Code:       "usage_limit_reached",
			Message:    "usage limit reached",
			HTTPStatus: http.StatusTooManyRequests,
			Retryable:  true,
		},
		Options: opts,
	})

	second, err := selector.Pick(
		context.Background(),
		"codex",
		"gpt-5.4",
		opts,
		[]*coreauth.Auth{authA, authB},
	)
	if err != nil {
		t.Fatalf("second Pick: %v", err)
	}
	if second == nil || second.ID != "b.json" {
		t.Fatalf("expected exhausted session binding to fail over to b.json, got %#v", second)
	}
}
