package auth

import (
	"context"
	"net/http"
	"testing"
	"time"
)

type authStateCaptureHook struct {
	NoopHook
	result Result
}

func (h *authStateCaptureHook) OnResult(_ context.Context, result Result) {
	h.result = result
}

func TestUpdateAggregatedAvailability_UnavailableWithoutNextRetryDoesNotBlockAuth(t *testing.T) {
	t.Parallel()

	now := time.Now()
	model := "test-model"
	auth := &Auth{
		ID: "a",
		ModelStates: map[string]*ModelState{
			model: {
				Status:      StatusError,
				Unavailable: true,
			},
		},
	}

	updateAggregatedAvailability(auth, now)

	if auth.Unavailable {
		t.Fatalf("auth.Unavailable = true, want false")
	}
	if !auth.NextRetryAfter.IsZero() {
		t.Fatalf("auth.NextRetryAfter = %v, want zero", auth.NextRetryAfter)
	}
}

func TestUpdateAggregatedAvailability_FutureNextRetryBlocksAuth(t *testing.T) {
	t.Parallel()

	now := time.Now()
	model := "test-model"
	next := now.Add(5 * time.Minute)
	auth := &Auth{
		ID: "a",
		ModelStates: map[string]*ModelState{
			model: {
				Status:         StatusError,
				Unavailable:    true,
				NextRetryAfter: next,
			},
		},
	}

	updateAggregatedAvailability(auth, now)

	if !auth.Unavailable {
		t.Fatalf("auth.Unavailable = false, want true")
	}
	if auth.NextRetryAfter.IsZero() {
		t.Fatalf("auth.NextRetryAfter = zero, want %v", next)
	}
	if auth.NextRetryAfter.Sub(next) > time.Second || next.Sub(auth.NextRetryAfter) > time.Second {
		t.Fatalf("auth.NextRetryAfter = %v, want %v", auth.NextRetryAfter, next)
	}
}

func TestManagerMarkResultReportsEffectiveSchedulerCooldown(t *testing.T) {
	t.Parallel()

	hook := &authStateCaptureHook{}
	manager := NewManager(nil, &RoundRobinSelector{}, hook)
	auth := &Auth{ID: "codex-auth", Provider: "codex", Metadata: map[string]any{"type": "codex"}}
	if _, err := manager.Register(context.Background(), auth); err != nil {
		t.Fatalf("register auth: %v", err)
	}

	startedAt := time.Now()
	manager.MarkResult(context.Background(), Result{
		AuthID:   auth.ID,
		Provider: auth.Provider,
		Model:    "gpt-5.5",
		Success:  false,
		Error: &Error{
			Code:       "auth_unavailable",
			Message:    "invalid or expired token",
			HTTPStatus: http.StatusUnauthorized,
		},
	})

	result := hook.result
	if !result.AuthStateKnown || result.AuthAvailable {
		t.Fatalf("auth state = known:%v available:%v, want known unavailable", result.AuthStateKnown, result.AuthAvailable)
	}
	if result.AuthStateReason != "unauthorized" {
		t.Fatalf("AuthStateReason = %q, want unauthorized", result.AuthStateReason)
	}
	if result.NextRetryAt.Before(startedAt.Add(29 * time.Minute)) {
		t.Fatalf("NextRetryAt = %v, want approximately 30 minutes", result.NextRetryAt)
	}
}

func TestManagerResetAuthStateClearsSchedulerCooldown(t *testing.T) {
	t.Parallel()

	manager := NewManager(nil, &RoundRobinSelector{}, nil)
	auth := &Auth{ID: "codex-reset", Provider: "codex", Metadata: map[string]any{"type": "codex"}}
	if _, err := manager.Register(context.Background(), auth); err != nil {
		t.Fatalf("register auth: %v", err)
	}
	manager.MarkResult(context.Background(), Result{
		AuthID:   auth.ID,
		Provider: auth.Provider,
		Model:    "gpt-5.5",
		Success:  false,
		Error: &Error{
			Code:       "auth_unavailable",
			Message:    "invalid or expired token",
			HTTPStatus: http.StatusUnauthorized,
		},
	})

	reset, err := manager.ResetAuthState(context.Background(), auth.ID)
	if err != nil {
		t.Fatalf("reset auth: %v", err)
	}
	if reset == nil {
		t.Fatal("reset auth should be returned")
	}
	if reset.Unavailable || !reset.NextRetryAfter.IsZero() || reset.LastError != nil {
		t.Fatalf("reset auth still blocked: %#v", reset)
	}
	if len(reset.ModelStates) != 0 {
		t.Fatalf("ModelStates = %#v, want empty", reset.ModelStates)
	}
}
