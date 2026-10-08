package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestQuotaCooldownSelectorPreservesExhaustedAccounts(t *testing.T) {
	manager := coreauth.NewManager(nil, &coreauth.RoundRobinSelector{}, nil)
	if _, err := manager.Register(context.Background(), &coreauth.Auth{
		ID:       "auth-quota.json",
		Provider: "codex",
		Status:   coreauth.StatusActive,
	}); err != nil {
		t.Fatalf("register auth: %v", err)
	}
	account := &accountSpec{
		ID:            "account-quota",
		AuthID:        "auth-quota.json",
		AuthKind:      "oauth",
		QuotaCooldown: &quotaCooldownState{Exhausted: true, UpdatedAtMS: time.Now().UnixMilli()},
	}
	m := &manifest{
		Accounts:        []accountSpec{*account},
		accountByID:     map[string]*accountSpec{"account-quota": account},
		accountByAuthID: map[string]*accountSpec{"auth-quota.json": account},
		quotaCooldowns:  newQuotaCooldownStateStore("", nil),
		authManager:     manager,
	}
	m.quotaCooldowns.snapshot.Store(map[string]quotaCooldownState{
		"account-quota": {Exhausted: true, UpdatedAtMS: time.Now().UnixMilli()},
	})
	selector := &quotaCooldownSelector{
		manifest: m,
		fallback: &cockpitSelector{manifest: m},
	}
	auth := &coreauth.Auth{ID: "auth-quota.json", Provider: "codex", Status: coreauth.StatusActive}

	selected, err := selector.Pick(context.Background(), "codex", "gpt-5.5", cliproxyexecutor.Options{}, []*coreauth.Auth{auth})
	if err == nil || selected != nil {
		t.Fatalf("exhausted account must remain unavailable, got auth=%#v err=%v", selected, err)
	}
	if !accountQuotaExhausted(m, account, time.Now()) {
		t.Fatal("automatic recovery must preserve the authoritative quota cooldown")
	}
}

func TestQuotaCooldownFreshManual100SurvivesStaleZero(t *testing.T) {
	zero := 0
	account := &accountSpec{ID: "manual-quota", AuthKind: "oauth", RemainingQuota: &zero}
	m := &manifest{accountByID: map[string]*accountSpec{account.ID: account}}
	path := filepath.Join(t.TempDir(), "quota-pool-state.json")
	m.quotaCooldowns = newQuotaCooldownStateStore(path, nil)
	write := func(percent int, sampled int64) {
		body := fmt.Sprintf(`{"accounts":{"manual-quota":{"primary":{"remainingPercent":%d},"cooldown":{"exhausted":%t,"updatedAtMs":%d}}}}`, percent, percent == 0, sampled)
		if err := os.WriteFile(path, []byte(body), 0600); err != nil {
			t.Fatal(err)
		}
		if err := m.quotaCooldowns.load(); err != nil {
			t.Fatal(err)
		}
	}
	write(0, 1000)
	if !accountQuotaExhausted(m, account, time.Now()) {
		t.Fatal("initial zero observation was not blocked")
	}
	// Explicit user recovery is authoritative, not an automatic-recovery bug.
	clearQuotaCooldownForAccounts(m, []string{account.ID}, time.UnixMilli(2000))
	write(100, 3000)
	if accountQuotaExhausted(m, account, time.Now()) {
		t.Fatal("fresh manual 100 did not restore eligibility")
	}
	write(0, 1000)
	if accountQuotaExhausted(m, account, time.Now()) {
		t.Fatal("stale zero overwrote fresh manual 100")
	}
}

func TestQuotaCooldownMissingEntryPreservesExhaustionUntilFreshRecovery(t *testing.T) {
	account := &accountSpec{ID: "account", AuthID: "account.json", AuthKind: "oauth"}
	m := &manifest{Accounts: []accountSpec{*account}, accountByID: map[string]*accountSpec{account.ID: account}}
	path := filepath.Join(t.TempDir(), "quota-pool-state.json")
	m.quotaCooldowns = newQuotaCooldownStateStore(path, m)
	write := func(body string) {
		t.Helper()
		if err := os.WriteFile(path, []byte(body), 0600); err != nil {
			t.Fatal(err)
		}
		if err := m.quotaCooldowns.load(); err != nil {
			t.Fatal(err)
		}
	}
	write(fmt.Sprintf(`{"accounts":{%q:{"primary":{"remainingPercent":0},"cooldown":{"exhausted":true,"updatedAtMs":1000}}}}`, account.ID))
	write(`{"accounts":{}}`)
	if !accountQuotaExhausted(m, account, time.Now()) {
		t.Fatal("missing observation reopened a known exhausted account")
	}
	observation := m.quotaCooldowns.snapshot.Load().(map[string]quotaCooldownState)[account.ID]
	if !observation.Exhausted || observation.UpdatedAtMS != 1000 {
		t.Fatalf("missing entry rolled back exhaustion weight: %+v", observation)
	}
	write(fmt.Sprintf(`{"accounts":{%q:{"primary":{"remainingPercent":80},"cooldown":{"exhausted":false,"updatedAtMs":2000}}}}`, account.ID))
	if accountQuotaExhausted(m, account, time.Now()) {
		t.Fatal("retained observation blocked a newer confirmed quota recovery")
	}
	observation = m.quotaCooldowns.snapshot.Load().(map[string]quotaCooldownState)[account.ID]
	if observation.Exhausted || observation.UpdatedAtMS != 2000 {
		t.Fatalf("fresh recovery did not refresh the retained weight: %+v", observation)
	}
}

func TestQuotaCooldownSelectorDoesNotRecoverDisabledAccounts(t *testing.T) {
	manager := coreauth.NewManager(nil, &coreauth.RoundRobinSelector{}, nil)
	account := &accountSpec{ID: "account-disabled", AuthID: "auth-disabled.json", AuthKind: "oauth"}
	m := &manifest{
		Accounts:        []accountSpec{*account},
		accountByID:     map[string]*accountSpec{"account-disabled": account},
		accountByAuthID: map[string]*accountSpec{"auth-disabled.json": account},
		authManager:     manager,
	}
	selector := &quotaCooldownSelector{
		manifest: m,
		fallback: &cockpitSelector{manifest: m},
	}
	auth := &coreauth.Auth{
		ID:       "auth-disabled.json",
		Provider: "codex",
		Status:   coreauth.StatusDisabled,
		Disabled: true,
	}
	if _, err := selector.Pick(context.Background(), "codex", "gpt-5.5", cliproxyexecutor.Options{}, []*coreauth.Auth{auth}); err == nil {
		t.Fatal("disabled accounts must stay unrecoverable")
	}
}

func TestPoolMemberRecoverableReasonAllowsQuotaCooldown(t *testing.T) {
	if !poolMemberRecoverableReason("quota_cooldown") {
		t.Fatal("quota_cooldown should be recoverable")
	}
	if !poolMemberRecoverableReason("account_cooldown") {
		t.Fatal("account_cooldown should be recoverable")
	}
	if poolMemberRecoverableReason("disabled") || poolMemberRecoverableReason("quota_reserved") {
		t.Fatal("disabled and quota_reserved must stay unrecoverable")
	}
}

func TestAutoRecoveryHonorsRuntimeDeadlinesAndDisabledModels(t *testing.T) {
	now := time.Now()
	for _, tc := range []struct {
		name      string
		configure func(*coreauth.Auth)
		want      bool
	}{
		{"future retry", func(a *coreauth.Auth) { a.Unavailable = true; a.NextRetryAfter = now.Add(time.Hour) }, false},
		{"unknown quota reset", func(a *coreauth.Auth) { a.Quota.Exceeded = true }, false},
		{"future model cooldown", func(a *coreauth.Auth) {
			a.ModelStates = map[string]*coreauth.ModelState{"other-model": {Unavailable: true, NextRetryAfter: now.Add(time.Hour)}}
		}, false},
		{"disabled model", func(a *coreauth.Auth) {
			a.ModelStates = map[string]*coreauth.ModelState{"other-model": {Status: coreauth.StatusDisabled}}
		}, false},
		{"expired retry", func(a *coreauth.Auth) { a.Unavailable = true; a.NextRetryAfter = now.Add(-time.Hour) }, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			manager := coreauth.NewManager(nil, &coreauth.RoundRobinSelector{}, nil)
			auth := &coreauth.Auth{ID: "auth-1", Provider: "codex", Status: coreauth.StatusError}
			tc.configure(auth)
			if _, err := manager.Register(context.Background(), auth); err != nil {
				t.Fatal(err)
			}
			m := &manifest{authManager: manager}
			got := recoverRuntimeAuths(context.Background(), m, "gpt-5.5", []*coreauth.Auth{auth})
			if (got > 0) != tc.want {
				t.Fatalf("recovered=%d, want %v", got, tc.want)
			}
			stored, _ := manager.GetByID(auth.ID)
			if !tc.want && (stored.Status != coreauth.StatusError || auth.Status != coreauth.StatusError) {
				t.Fatal("blocked runtime state was reset")
			}
		})
	}
}

func TestQuotaCooldownSelectorUsesHealthyAlternativeAndReset(t *testing.T) {
	now := time.Now()
	future := now.Add(time.Hour).UnixMilli()
	past := now.Add(-time.Hour).UnixMilli()
	for _, tc := range []struct {
		name     string
		cooldown quotaCooldownState
		wantID   string
	}{
		{"blocked", quotaCooldownState{Exhausted: true, ResetAtMS: &future}, "auth-api"},
		{"reset elapsed", quotaCooldownState{Exhausted: true, ResetAtMS: &past}, "auth-oauth"},
		{"fresh positive quota", quotaCooldownState{UpdatedAtMS: now.UnixMilli()}, "auth-oauth"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			account := &accountSpec{ID: "oauth", AuthID: "auth-oauth", AuthKind: "oauth", QuotaCooldown: &tc.cooldown}
			api := &accountSpec{ID: "api", AuthID: "auth-api", AuthKind: "api_key"}
			m := &manifest{accountByAuthID: map[string]*accountSpec{"auth-oauth": account, "auth-api": api}}
			s := &quotaCooldownSelector{manifest: m, fallback: &cockpitSelector{manifest: m}}
			selected, err := s.Pick(context.Background(), "codex", "gpt-5.5", cliproxyexecutor.Options{}, []*coreauth.Auth{
				{ID: "auth-oauth", Provider: "codex", Status: coreauth.StatusActive},
				{ID: "auth-api", Provider: "codex", Status: coreauth.StatusActive},
			})
			if err != nil || selected == nil || selected.ID != tc.wantID {
				t.Fatalf("selected=%v err=%v want=%s", selected, err, tc.wantID)
			}
		})
	}
}

func TestLegacyCooldownKeepsUnknownExhaustedWindow(t *testing.T) {
	zero := 0
	future := time.Now().Add(time.Hour).Unix()
	unknown := &quotaPoolWindowState{RemainingPercent: &zero}
	known := &quotaPoolWindowState{RemainingPercent: &zero, ResetAt: &future}
	for _, windows := range [][2]*quotaPoolWindowState{{unknown, known}, {known, unknown}} {
		cooldown := legacyQuotaCooldownFromPoolState(quotaPoolAccountState{Primary: windows[0], Secondary: windows[1]})
		if !cooldown.Exhausted || cooldown.ResetAtMS != nil {
			t.Fatalf("unknown exhausted window lost: %#v", cooldown)
		}
	}
}

func TestQuotaCooldownRecoveryBoundaries(t *testing.T) {
	for _, scenario := range []string{"future-reset", "unknown-reset", "expired-reset", "fresh-healthy", "manual-clear", "transient-only"} {
		t.Run(scenario, func(t *testing.T) {
			now := time.Now()
			reset := now.Add(time.Hour).UnixMilli()
			state := quotaCooldownState{Exhausted: true, ResetAtMS: &reset, UpdatedAtMS: now.UnixMilli()}
			wantBlocked := scenario == "future-reset" || scenario == "unknown-reset"
			switch scenario {
			case "unknown-reset":
				state.ResetAtMS = nil
			case "expired-reset":
				reset = now.Add(-time.Second).UnixMilli()
			case "fresh-healthy", "transient-only":
				state.Exhausted = false
			}
			account := &accountSpec{ID: "fixture-recovery", AuthID: "fixture-recovery.json", AuthKind: "oauth", QuotaCooldown: &state}
			m := &manifest{Accounts: []accountSpec{*account}, accountByID: map[string]*accountSpec{account.ID: account}, accountByAuthID: map[string]*accountSpec{account.AuthID: account}}
			m.quotaCooldowns = newQuotaCooldownStateStore("", m)
			m.authManager = coreauth.NewManager(nil, &coreauth.RoundRobinSelector{}, nil)
			auth := &coreauth.Auth{ID: account.AuthID, Provider: "codex", Status: coreauth.StatusActive, Unavailable: true, NextRetryAfter: now.Add(-time.Minute)}
			if _, err := m.authManager.Register(context.Background(), auth); err != nil {
				t.Fatal(err)
			}
			if scenario == "manual-clear" {
				clearQuotaCooldownForAccounts(m, []string{account.ID}, now)
			}
			before := m.quotaCooldowns.snapshot.Load().(map[string]quotaCooldownState)[account.ID]
			selector := &quotaCooldownSelector{manifest: m, fallback: &cockpitSelector{manifest: m}}
			picked, err := selector.Pick(context.Background(), "codex", "gpt-5.5", cliproxyexecutor.Options{}, []*coreauth.Auth{auth})
			if wantBlocked {
				if err == nil || picked != nil || !auth.Unavailable {
					t.Fatalf("exhaustion must block runtime recovery, picked=%v err=%v", picked != nil, err)
				}
			} else if err != nil || picked == nil || !authAvailable(auth, "gpt-5.5", now) {
				t.Fatalf("transient recovery should remain available, picked=%v err=%v", picked != nil, err)
			}
			after := m.quotaCooldowns.snapshot.Load().(map[string]quotaCooldownState)[account.ID]
			if before != after {
				t.Fatal("runtime recovery rewrote authoritative quota observation")
			}
		})
	}
}
