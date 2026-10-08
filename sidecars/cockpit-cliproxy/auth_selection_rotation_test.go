package main

import (
	"context"
	"fmt"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func rotationSelectorForTest() (*cockpitSelector, []*coreauth.Auth) {
	m := &manifest{RoutingStrategy: "auto", accountByID: map[string]*accountSpec{}, accountByAuthID: map[string]*accountSpec{}, originalIndexByID: map[string]int{}}
	var auths []*coreauth.Auth
	for index, id := range []string{"a", "b", "c"} {
		account := &accountSpec{ID: id, AuthID: id + ".json"}
		m.Accounts = append(m.Accounts, *account)
		m.accountByID[id], m.accountByAuthID[account.AuthID], m.originalIndexByID[id] = account, account, index
		auths = append(auths, &coreauth.Auth{ID: account.AuthID, Provider: "codex", Status: coreauth.StatusActive})
	}
	return &cockpitSelector{manifest: m}, auths
}

func rotationContextForTest(key string, scope ...string) context.Context {
	return context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{ID: key, AccountIDs: scope})
}

func expectRotationPick(t *testing.T, selector *cockpitSelector, ctx context.Context, model string, auths []*coreauth.Auth, want string) {
	t.Helper()
	auth, err := selector.Pick(ctx, "codex", model, cliproxyexecutor.Options{}, auths)
	if err != nil || auth == nil || auth.ID != want+".json" {
		t.Fatalf("Pick = %#v, %v; want %s", auth, err, want)
	}
}

func TestRoutingIdentityCursorSparseScope(t *testing.T) {
	s, auths := rotationSelectorForTest()
	ctx := rotationContextForTest("key", "a", "c")
	for _, want := range []string{"a", "c", "a", "c", "a", "c"} {
		expectRotationPick(t, s, ctx, "gpt-5.4", auths, want)
	}
}

func TestRoutingIdentityCursorRemovedAccountAndRecovery(t *testing.T) {
	s, auths := rotationSelectorForTest()
	ctx := rotationContextForTest("key", "a", "b", "c")
	expectRotationPick(t, s, ctx, "gpt-5.4", auths, "a")
	auths[0].Disabled = true
	expectRotationPick(t, s, ctx, "gpt-5.4", auths, "b")
	auths[0].Disabled = false
	expectRotationPick(t, s, ctx, "gpt-5.4", auths, "c")
	expectRotationPick(t, s, ctx, "gpt-5.4", auths, "a")
	// Removing the previous winner must continue after its stable position.
	expectRotationPick(t, s, ctx, "gpt-5.4", auths[1:], "b")
	expectRotationPick(t, s, ctx, "gpt-5.4", auths, "c")
}

func TestRoutingIdentityCursorSeparatesKeyModelAndPersistentScope(t *testing.T) {
	s, auths := rotationSelectorForTest()
	first := rotationContextForTest("first", "a", "c")
	second := rotationContextForTest("second", "a", "c")
	expectRotationPick(t, s, first, "gpt-5.4", auths, "a")
	expectRotationPick(t, s, second, "gpt-5.4", auths, "a")
	expectRotationPick(t, s, first, "gpt-5.5", auths, "a")
	expectRotationPick(t, s, first, "gpt-5.4", auths, "c")
	// Reordered/duplicated persistent IDs represent the same authorization scope.
	expectRotationPick(t, s, rotationContextForTest("first", "c", "a", "a"), "gpt-5.4", auths, "a")
	expectRotationPick(t, s, rotationContextForTest("first"), "gpt-5.4", auths, "a")
	expectRotationPick(t, s, rotationContextForTest("first", "a", "b", "c"), "gpt-5.4", auths, "a")
	expectRotationPick(t, s, second, "gpt-5.4", auths, "c")
}

func TestRoutingIdentityCursorPreservesRankAndCustomWeights(t *testing.T) {
	s, auths := rotationSelectorForTest()
	rank := 5
	s.manifest.accountByID["a"].PlanRank = &rank
	for range 5 {
		expectRotationPick(t, s, context.Background(), "gpt-5.4", auths, "a")
	}
	s.manifest.RoutingStrategy = "custom"
	s.manifest.CustomRoutingRules = []customRoutingRule{{AccountID: "a", Weight: 2}, {AccountID: "b", Weight: 1}, {AccountID: "c", Weight: 1}}
	s.cursor = 0
	for _, want := range []string{"a", "a", "b", "c", "a"} {
		expectRotationPick(t, s, context.Background(), "gpt-5.4", auths, want)
	}
}

func TestRoutingIdentityCursorBoundedAndConcurrent(t *testing.T) {
	s, auths := rotationSelectorForTest()
	for index := range maxRoutingRotationCursors + 10 {
		expectRotationPick(t, s, rotationContextForTest(fmt.Sprint(index)), "gpt-5.4", auths, "a")
	}
	if len(s.rotationCursors) != maxRoutingRotationCursors || len(s.rotationCursorOrder) != maxRoutingRotationCursors {
		t.Fatalf("rotation namespace is unbounded: %d / %d", len(s.rotationCursors), len(s.rotationCursorOrder))
	}
	for index := range 30 {
		t.Run(fmt.Sprint(index), func(t *testing.T) {
			t.Parallel()
			expectRotationPick(t, s, rotationContextForTest(fmt.Sprintf("parallel-%s", t.Name())), "gpt-5.4", auths, "a")
		})
	}
}

func TestRoutingIdentityCursorConcurrentSameScope(t *testing.T) {
	s, auths := rotationSelectorForTest()
	ctx := rotationContextForTest("shared", "a", "b", "c")
	results := make(chan string, 30)
	for range 30 {
		go func() {
			auth, err := s.Pick(ctx, "codex", "gpt-5.4", cliproxyexecutor.Options{}, auths)
			if err != nil || auth == nil {
				results <- fmt.Sprint(err)
				return
			}
			results <- auth.ID
		}()
	}
	counts := make(map[string]int)
	for range 30 {
		counts[<-results]++
	}
	for _, auth := range auths {
		if counts[auth.ID] != 10 {
			t.Fatalf("concurrent turns were lost: %#v", counts)
		}
	}
}
