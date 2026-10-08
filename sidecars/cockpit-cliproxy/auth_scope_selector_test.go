package main

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestAPIKeyScopePrecedesBackupFallback(t *testing.T) {
	s, auths := rotationSelectorForTest()
	s.manifest.CustomRoutingRules = []customRoutingRule{{AccountID: "c", IsBackup: true}}
	selector := buildCoreAuthSelector(nil, s, s.manifest, nil)
	auth, err := selector.Pick(rotationContextForTest("key", "c"), "codex", "gpt-5.4", cliproxyexecutor.Options{}, auths)
	if err != nil || auth == nil || auth.ID != "c.json" {
		t.Fatalf("outside-scope regular accounts suppressed authorized backup: auth=%#v err=%v", auth, err)
	}
}

func TestAPIKeyScopePrecedesQuotaReserve(t *testing.T) {
	s, auths := rotationSelectorForTest()
	threshold, remaining, present, updated := 20, 10, true, time.Now().Unix()
	s.manifest.accountByID["a"].QuotaReserve = &quotaReserveSpec{
		HourlyThresholdPercent: &threshold, HourlyRemainingPercent: &remaining,
		HourlyWindowPresent: &present, SnapshotUpdatedAtUnixSeconds: &updated,
	}
	selector := buildCoreAuthSelector(nil, s, s.manifest, nil)
	auth, err := selector.Pick(rotationContextForTest("key", "a"), "codex", "gpt-5.4", cliproxyexecutor.Options{}, auths)
	if auth != nil || err == nil || !strings.Contains(err.Error(), "quota reserve blocked 1 auth(s)") {
		t.Fatalf("authorized reserve rejection became a scope mismatch: auth=%#v err=%v", auth, err)
	}
}

func TestAPIKeyScopePrecedesReserveFailureAndPreservesUnrestricted(t *testing.T) {
	s, auths := rotationSelectorForTest()
	threshold, remaining, present, updated := 20, 10, true, time.Now().Unix()
	s.manifest.accountByID["a"].QuotaReserve = &quotaReserveSpec{
		HourlyThresholdPercent: &threshold, HourlyRemainingPercent: &remaining,
		HourlyWindowPresent: &present, SnapshotUpdatedAtUnixSeconds: &updated,
	}
	selector := buildCoreAuthSelector(nil, s, s.manifest, nil)
	auth, err := selector.Pick(rotationContextForTest("key", "missing"), "codex", "gpt-5.4", cliproxyexecutor.Options{}, auths[:1])
	if auth != nil || err == nil || strings.Contains(err.Error(), "quota reserve blocked") {
		t.Fatalf("outside-scope reserve account influenced failure: auth=%#v err=%v", auth, err)
	}
	var diagnostic *coreauth.Error
	if !errors.As(err, &diagnostic) || diagnostic.Code != "auth_unavailable" || diagnostic.HTTPStatus != 503 || !diagnostic.Retryable {
		t.Fatalf("scope error lost diagnostic contract: %v", err)
	}
	auth, err = selector.Pick(context.Background(), "codex", "gpt-5.4", cliproxyexecutor.Options{}, auths)
	if err != nil || auth == nil || auth.ID != "b.json" {
		t.Fatalf("unrestricted routing changed: auth=%#v err=%v", auth, err)
	}
}
