package auth

import (
	"context"
	"testing"
	"time"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

type transientRequestTestError struct{ customStatusError }

func (transientRequestTestError) IsTransientRequestScoped() bool { return true }

func TestLegacyTransientMarkerDoesNotBypassUpstreamCooldown(t *testing.T) {
	err := transientRequestTestError{overloadStatusError()}
	result := resultErrorFromError(err)
	if result.Code == "transient_request_scoped" || shouldSkipCredentialCooldown(result) {
		t.Fatal("legacy marker must not bypass CLIProxyAPI cooldown rules")
	}
}

func TestUpstreamOverloadRetryKeepsRoundAndWaitBudgets(t *testing.T) {
	m := NewManager(nil, nil, nil)
	ids := registerOverloadAuths(t, m, 1)
	for _, tc := range []struct {
		rounds, attempt int
		maxWait         time.Duration
		cooled, retry   bool
	}{
		{1, 0, 0, false, true},
		{1, 1, 2 * time.Minute, false, false},
		{0, 0, 2 * time.Minute, false, false},
		{1, 0, 2 * time.Minute, true, true},
		{1, 0, 0, true, false},
		{1, 0, time.Second, true, false},
	} {
		m.SetRetryConfig(tc.rounds, tc.maxWait, 1)
		m.mu.Lock()
		auth := m.auths[ids[0]]
		auth.Unavailable = tc.cooled
		auth.LastError = &Error{HTTPStatus: 503, Message: "overloaded"}
		auth.NextRetryAfter = time.Time{}
		if tc.cooled {
			auth.NextRetryAfter = time.Now().Add(time.Minute)
		}
		m.mu.Unlock()
		wait, retry := m.shouldRetryAfterErrorWithAttempted(context.Background(), cliproxyexecutor.Options{}, overloadStatusError(), tc.attempt,
			[]string{"codex"}, "gpt-5.6-terra", tc.maxWait, 0, tc.rounds, map[string]struct{}{ids[0]: {}})
		if retry != tc.retry {
			t.Fatalf("rounds=%d attempt=%d maxWait=%v cooled=%v: wait=%v retry=%v", tc.rounds, tc.attempt, tc.maxWait, tc.cooled, wait, retry)
		}
		if retry && tc.cooled && (wait < 55*time.Second || wait > time.Minute) {
			t.Fatalf("credential cooldown not respected: %v", wait)
		}
		if retry && !tc.cooled && wait != 0 {
			t.Fatalf("unexpected local transient delay: %v", wait)
		}
	}
}
