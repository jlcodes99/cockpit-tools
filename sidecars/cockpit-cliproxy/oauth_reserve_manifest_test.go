package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestOAuthReserveManifestPolicyCanBeEnabledAndDisabled(t *testing.T) {
	now := time.Now()
	for _, tt := range []struct {
		name    string
		reserve string
		blocked bool
	}{
		{name: "enabled below reserve", reserve: fmt.Sprintf(`,"quotaReserve":{"hourlyThresholdPercent":10,"weeklyThresholdPercent":10,"hourlyRemainingPercent":1,"weeklyRemainingPercent":50,"hourlyWindowPresent":true,"weeklyWindowPresent":true,"snapshotUpdatedAtUnixSeconds":%d,"snapshotVerifiedQueryStartedAtUnixMillis":%d}`, now.Unix(), now.UnixMilli()), blocked: true},
		{name: "enabled above reserve", reserve: fmt.Sprintf(`,"quotaReserve":{"hourlyThresholdPercent":10,"weeklyThresholdPercent":10,"hourlyRemainingPercent":50,"weeklyRemainingPercent":50,"hourlyWindowPresent":true,"weeklyWindowPresent":true,"snapshotUpdatedAtUnixSeconds":%d,"snapshotVerifiedQueryStartedAtUnixMillis":%d}`, now.Unix(), now.UnixMilli())},
		{name: "disabled", reserve: ""},
	} {
		t.Run(tt.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "manifest.json")
			raw := fmt.Sprintf(`{"accounts":[{"id":"oauth-fixture","email":"fixture@example.invalid","authId":"oauth-fixture.json","remainingQuota":1%s}]}`, tt.reserve)
			if err := os.WriteFile(path, []byte(raw), 0600); err != nil {
				t.Fatal(err)
			}
			m, err := loadManifest(path)
			if err != nil {
				t.Fatal(err)
			}
			selector := buildCoreAuthSelector(nil, &cockpitSelector{manifest: m}, m, nil)
			if stoppable, ok := selector.(coreauth.StoppableSelector); ok {
				defer stoppable.Stop()
			}
			account := &coreauth.Auth{ID: "oauth-fixture.json", Provider: "codex", Status: coreauth.StatusActive}
			ctx, cancel := context.WithCancel(accountConcurrencyRequestContext(tt.name))
			defer cancel()
			selected, err := selector.Pick(ctx, "codex", "gpt-5.4", cliproxyexecutor.Options{}, []*coreauth.Auth{account})
			if tt.blocked {
				if selected != nil || err == nil || !strings.Contains(err.Error(), "quota reserve") {
					t.Fatalf("reserve did not stop routing: selected=%v err=%v", selected, err)
				}
			} else if err != nil || selected == nil || selected.ID != account.ID {
				t.Fatalf("eligible account rejected: selected=%v err=%v", selected, err)
			}
			encoded, err := json.Marshal(m.Accounts[0])
			if err != nil {
				t.Fatal(err)
			}
			if strings.Contains(string(encoded), "quotaReserve") != (tt.reserve != "") {
				t.Fatalf("reserve persistence changed: %s", encoded)
			}
		})
	}
}

func TestDisablingOAuthReserveDoesNotAdmitDisabledAccount(t *testing.T) {
	account := &accountSpec{ID: "fixture", AuthID: "fixture.json"}
	m := &manifest{Accounts: []accountSpec{*account}, accountByID: map[string]*accountSpec{account.ID: account}, accountByAuthID: map[string]*accountSpec{account.AuthID: account}}
	selector := buildCoreAuthSelector(nil, &cockpitSelector{manifest: m}, m, nil)
	auth := &coreauth.Auth{ID: account.AuthID, Provider: "codex", Status: coreauth.StatusActive, Disabled: true}
	selected, err := selector.Pick(accountConcurrencyRequestContext("disabled"), "codex", "gpt-5.4", cliproxyexecutor.Options{}, []*coreauth.Auth{auth})
	if err == nil || selected != nil {
		t.Fatal("disabling reserve admitted a disabled account")
	}
}
