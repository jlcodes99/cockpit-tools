package main

import (
	"context"
	"errors"
	"reflect"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestPausedManifestRetainsMembersAndExplicitScope(t *testing.T) {
	m := loadAutomaticRoutingManifest(t, `{"pausedAccountIds":[" a ","a",""],"accounts":[{"id":"a","authId":"a.json"},{"id":"b","authId":"b.json"}],"apiKeys":[{"id":"scope","key":"key","enabled":true,"accountIds":["a"]}]}`)
	if !reflect.DeepEqual(m.PausedAccountIDs, []string{"a"}) || len(m.Accounts) != 2 || !reflect.DeepEqual(m.APIKeys[0].AccountIDs, []string{"a"}) {
		t.Fatalf("pause changed membership or explicit scope: %#v", m)
	}
	base := &cockpitSelector{manifest: m}
	selector := buildCoreAuthSelector(nil, base, m, nil)
	auths := []*coreauth.Auth{testAuth("a.json"), testAuth("b.json")}
	ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &m.APIKeys[0])
	selected, err := selector.Pick(ctx, "codex", "", cliproxyexecutor.Options{}, auths)
	var typed *coreauth.Error
	if selected != nil || !errors.As(err, &typed) || typed.Code != "account_paused" || typed.HTTPStatus != 503 || !coreauth.IsTerminalAuthError(err) {
		t.Fatalf("all-paused explicit scope must fail closed: selected=%#v err=%v", selected, err)
	}
	if auths[0].Disabled || len(m.Accounts) != 2 || !reflect.DeepEqual(m.APIKeys[0].AccountIDs, []string{"a"}) {
		t.Fatal("selection mutated shared credentials or scope")
	}
}

func TestPausedAccountIDsKeepCaseSensitiveIdentity(t *testing.T) {
	m := loadAutomaticRoutingManifest(t, `{"pausedAccountIds":[" A ","a","A",""],"accounts":[{"id":"A"},{"id":"a"}]}`)
	if !reflect.DeepEqual(m.PausedAccountIDs, []string{"A", "a"}) || !serviceAccountPaused(nil, m, "A") || !serviceAccountPaused(nil, m, "a") {
		t.Fatalf("pause conflated case-sensitive account identities: %v", m.PausedAccountIDs)
	}
}

func TestPausedAccountTerminalMarkerRequiresExplicitMatchingScope(t *testing.T) {
	base, auths := rotationSelectorForTest()
	base.manifest.PausedAccountIDs = []string{"a", "b", "c"}
	selector := buildCoreAuthSelector(nil, base, base.manifest, nil)
	for name, ctx := range map[string]context.Context{
		"unrestricted":   rotationContextForTest("key"),
		"scope mismatch": rotationContextForTest("key", "missing"),
	} {
		selected, err := selector.Pick(ctx, "codex", "", cliproxyexecutor.Options{}, auths)
		if selected != nil || err == nil || coreauth.IsTerminalAuthError(err) {
			t.Fatalf("%s was incorrectly classified as terminal scoped pause: %#v %v", name, selected, err)
		}
	}
}

func TestPausedAccountPrecedesBackupAndAffinity(t *testing.T) {
	base, auths := rotationSelectorForTest()
	m := base.manifest
	m.CustomRoutingRules = []customRoutingRule{{AccountID: "b", IsBackup: true}}
	cfg := &config.Config{}
	cfg.Routing.SessionAffinity = true
	selector := buildCoreAuthSelector(cfg, &orderedAuthSelector{order: []string{"a.json", "b.json"}}, m, nil)
	t.Cleanup(func() { selector.(coreauth.StoppableSelector).Stop() })
	ctx := rotationContextForTest("scope", "a", "b")
	opts := cliproxyexecutor.Options{OriginalRequest: []byte(`{"model":"gpt-5.4","prompt_cache_key":"pause-affinity"}`)}
	selected, err := selector.Pick(ctx, "codex", "gpt-5.4", opts, auths)
	if err != nil || selected == nil || selected.ID != "a.json" {
		t.Fatalf("prime affinity: selected=%#v err=%v", selected, err)
	}
	// Configuration replacement is sequential here, as a service restart is in production.
	m.PausedAccountIDs = []string{"a"}
	selected, err = selector.Pick(ctx, "codex", "gpt-5.4", opts, auths)
	if err != nil || selected == nil || selected.ID != "b.json" {
		t.Fatalf("paused primary suppressed active backup or reused affinity: selected=%#v err=%v", selected, err)
	}
}

func TestPausedAccountIsServiceLocalAndInternalCallsRemainAvailable(t *testing.T) {
	base, auths := rotationSelectorForTest()
	base.manifest.PausedAccountIDs = []string{"a"}
	paused := buildCoreAuthSelector(nil, base, base.manifest, nil)
	other, _ := rotationSelectorForTest()
	active := buildCoreAuthSelector(nil, other, other.manifest, nil)
	ctx := rotationContextForTest("scope", "a")
	if selected, err := paused.Pick(ctx, "codex", "", cliproxyexecutor.Options{}, auths); selected != nil || err == nil {
		t.Fatalf("service pause ignored: %#v %v", selected, err)
	}
	for name, selector := range map[string]coreauth.Selector{"other service": active, "internal": paused} {
		requestCtx := ctx
		if name == "internal" {
			requestCtx = context.WithValue(ctx, clientAPIKeyContextKey, &apiKeySpec{Internal: true, AccountIDs: []string{"a"}})
		}
		selected, err := selector.Pick(requestCtx, "codex", "", cliproxyexecutor.Options{}, auths)
		if err != nil || selected == nil || selected.ID != "a.json" || auths[0].Disabled {
			t.Fatalf("pause leaked into %s: %#v %v", name, selected, err)
		}
	}
}

func TestPausedUpstreamGuardBlocksPinnedAuthWithoutMutatingScope(t *testing.T) {
	base, _ := rotationSelectorForTest()
	base.manifest.PausedAccountIDs = []string{"a"}
	policy := &requestPolicy{manifest: base.manifest}
	ctx := cliproxyexecutor.WithUpstreamSendGuard(rotationContextForTest("scope", "a", "b"), policy.beforeUpstreamSend)
	if err := cliproxyexecutor.CheckUpstreamSend(ctx, "a.json"); err == nil || !cliproxyexecutor.IsUpstreamSendGuardError(err) {
		t.Fatalf("paused pinned credential escaped final send guard: %v", err)
	}
	if err := cliproxyexecutor.CheckUpstreamSend(ctx, "b.json"); err != nil {
		t.Fatalf("active credential rejected: %v", err)
	}
	internal := context.WithValue(ctx, clientAPIKeyContextKey, &apiKeySpec{Internal: true, AccountIDs: []string{"a"}})
	if err := cliproxyexecutor.CheckUpstreamSend(internal, "a.json"); err != nil {
		t.Fatalf("internal background request rejected: %v", err)
	}
}
