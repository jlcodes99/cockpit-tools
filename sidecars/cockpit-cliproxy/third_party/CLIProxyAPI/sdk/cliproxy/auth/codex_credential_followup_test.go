package auth

import (
	"context"
	"errors"
	"net/http"
	"testing"
	"time"
)

type followupBlockingRefresh struct {
	schedulerProviderTestExecutor
	started chan struct{}
	release chan struct{}
	fail    bool
}

type followupResultHook struct {
	NoopHook
	results []Result
}

func (h *followupResultHook) OnResult(_ context.Context, result Result) {
	h.results = append(h.results, result)
}

func (e *followupBlockingRefresh) Refresh(ctx context.Context, auth *Auth) (*Auth, error) {
	close(e.started)
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-e.release:
	}
	if e.fail {
		return nil, errors.New("invalid_grant")
	}
	auth.Metadata["access_token"] = "late-refresh"
	auth.Metadata["refresh_token"] = "late-refresh-secret"
	return auth, nil
}

func TestCodexFollowupRefreshPreservesConcurrentCredentials(t *testing.T) {
	for _, fail := range []bool{false, true} {
		t.Run(map[bool]string{false: "success", true: "failure"}[fail], func(t *testing.T) {
			m := NewManager(nil, &RoundRobinSelector{}, nil)
			exec := &followupBlockingRefresh{schedulerProviderTestExecutor: schedulerProviderTestExecutor{provider: "codex"}, started: make(chan struct{}), release: make(chan struct{}), fail: fail}
			m.RegisterExecutor(exec)
			original, err := m.Register(t.Context(), &Auth{ID: "followup", Provider: "codex", Metadata: map[string]any{"access_token": "old", "refresh_token": "old-refresh"}})
			if err != nil {
				t.Fatal(err)
			}
			done := make(chan error, 1)
			go func() { _, err := m.refreshAuthForRequest(t.Context(), original.ID, "old"); done <- err }()
			<-exec.started
			current, _ := m.GetByID(original.ID)
			if current.HasValidAccessToken(time.Now()) {
				t.Fatal("rejected token remains selectable during refresh")
			}
			current.Metadata["access_token"] = "reauthorized"
			current.Metadata["refresh_token"] = "reauthorized-refresh"
			current, err = m.Update(t.Context(), current)
			if err != nil {
				t.Fatal(err)
			}
			close(exec.release)
			if err := <-done; err != nil {
				t.Fatal(err)
			}
			latest, _ := m.GetByID(original.ID)
			if authAccessToken(latest) != "reauthorized" || latest.Metadata["refresh_token"] != "reauthorized-refresh" || latest.LastError != nil || latest.CredentialVersion != current.CredentialVersion {
				t.Fatal("late refresh replaced or invalidated new credentials")
			}
		})
	}
}

func TestCodexFollowupLateResultsCannotInvalidateNewCredentials(t *testing.T) {
	for _, reRegister := range []bool{false, true} {
		hook := &followupResultHook{}
		m := NewManager(nil, &RoundRobinSelector{}, hook)
		old, _ := m.Register(t.Context(), &Auth{ID: "late-result", Provider: "codex", Metadata: map[string]any{"access_token": "old"}})
		newAuth := old.Clone()
		newAuth.Metadata["access_token"] = "new"
		if reRegister {
			_, _ = m.Register(t.Context(), newAuth)
		} else {
			_, _ = m.Update(t.Context(), newAuth)
		}
		for _, status := range []int{http.StatusUnauthorized, http.StatusTooManyRequests} {
			m.recordExecutionResult(t.Context(), Result{AuthID: old.ID, Provider: "codex", Model: "test", Error: &Error{HTTPStatus: status}, Success: false}, old, false)
		}
		latest, _ := m.GetByID(old.ID)
		if len(hook.results) != 2 || !hook.results[0].StaleCredential || !hook.results[1].StaleCredential {
			t.Fatal("stale requests lost their diagnostic observations")
		}
		if latest.Unavailable || latest.LastError != nil || len(latest.ModelStates) > 0 {
			t.Fatal("late execution result altered new credentials")
		}
		m.MarkResult(t.Context(), Result{AuthID: latest.ID, CredentialVersion: latest.CredentialVersion, RegistrationEpoch: latest.RegistrationEpoch, Model: "test", Error: &Error{HTTPStatus: 429}})
		latest, _ = m.GetByID(old.ID)
		if latest.ModelStates["test"] == nil || !latest.ModelStates["test"].Quota.Exceeded {
			t.Fatal("current result did not retain quota cooling")
		}
	}
}

func TestCodexFollowupRefreshKeepsQuotaCooldown(t *testing.T) {
	now := time.Now()
	until := now.Add(time.Hour)
	auth := &Auth{ModelStates: map[string]*ModelState{"test": {Unavailable: true, LastError: &Error{HTTPStatus: 401}, Quota: QuotaState{Exceeded: true, NextRecoverAt: until}, NextRetryAfter: until}}}
	if resumed := clearUnauthorizedModelStates(auth, now); len(resumed) != 0 {
		t.Fatal("quota-cooled model resumed")
	}
	state := auth.ModelStates["test"]
	if !state.Quota.Exceeded || !state.NextRetryAfter.Equal(until) || state.LastError != nil {
		t.Fatal("refresh cleared quota cooling or retained unauthorized error")
	}
}

func TestCodexFollowupCancelledRefreshCanRetry(t *testing.T) {
	m := NewManager(nil, &RoundRobinSelector{}, nil)
	exec := &followupBlockingRefresh{schedulerProviderTestExecutor: schedulerProviderTestExecutor{provider: "codex"}, started: make(chan struct{}), release: make(chan struct{})}
	m.RegisterExecutor(exec)
	original, _ := m.Register(t.Context(), &Auth{ID: "cancel", Provider: "codex", Metadata: map[string]any{"access_token": "old", "refresh_token": "refresh", "expired": time.Now().Add(24 * time.Hour).Format(time.RFC3339)}})
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan error, 1)
	go func() { _, err := m.refreshAuthForRequest(ctx, original.ID, "old"); done <- err }()
	<-exec.started
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatalf("expected cancellation, got %v", err)
	}
	latest, _ := m.GetByID(original.ID)
	if latest.NextRefreshAfter.IsZero() || latest.NextRefreshAfter.After(time.Now().Add(2*time.Second)) {
		t.Fatal("cancelled refresh was not rescheduled")
	}
	later := time.Now().Add(3 * time.Second)
	if next, scheduled := nextRefreshCheckAt(later, latest, time.Second); !scheduled || next.After(later) || !m.shouldRefresh(latest, later) {
		t.Fatal("rejected but unexpired token prevents a cancelled refresh from retrying")
	}
}

func TestCodexFollowupInvalidRefreshGrantCannotBeRevivedByLateSuccess(t *testing.T) {
	m := NewManager(nil, &RoundRobinSelector{}, nil)
	m.RegisterExecutor(unauthorizedRefreshTestExecutor{schedulerProviderTestExecutor{provider: "codex"}})
	auth, _ := m.Register(t.Context(), &Auth{ID: "revoked", Provider: "codex", Metadata: map[string]any{"access_token": "old", "refresh_token": "refresh"}})
	_, err := m.refreshAuthForRequest(t.Context(), auth.ID, "old")
	if err == nil {
		t.Fatal("expected invalid grant")
	}
	m.recordExecutionResult(t.Context(), Result{AuthID: auth.ID, Success: true}, auth, false)
	latest, _ := m.GetByID(auth.ID)
	if !latest.Unavailable || latest.LastError == nil || latest.HasValidAccessToken(time.Now()) {
		t.Fatal("late success revived revoked credentials")
	}
}
