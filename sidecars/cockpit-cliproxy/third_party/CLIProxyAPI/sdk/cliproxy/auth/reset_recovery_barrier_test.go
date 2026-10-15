package auth

import (
	"context"
	"testing"
	"time"
)

func TestResetAuthStateIgnoresResultsFromOlderAttempts(t *testing.T) {
	manager := NewManager(nil, nil, nil)
	const authID = "recovery-barrier-auth"
	const model = "gpt-5.5"

	if _, err := manager.Register(context.Background(), &Auth{
		ID:       authID,
		Provider: "codex",
		Status:   StatusActive,
	}); err != nil {
		t.Fatalf("register auth: %v", err)
	}

	oldAttempt := newUpstreamAttemptContext(context.Background())
	if _, err := manager.ResetAuthState(context.Background(), authID); err != nil {
		t.Fatalf("reset auth state: %v", err)
	}

	manager.MarkResult(oldAttempt, Result{
		AuthID:   authID,
		Provider: "codex",
		Model:    model,
		Success:  false,
		Error: &Error{
			Code:       "upstream_error",
			Message:    "old request failed after recovery",
			HTTPStatus: 503,
		},
	})

	afterOldResult, ok := manager.GetByID(authID)
	if !ok || afterOldResult == nil {
		t.Fatal("auth missing after recovery")
	}
	if len(afterOldResult.ModelStates) != 0 || afterOldResult.Unavailable {
		t.Fatalf("old result restored cleared state: %#v", afterOldResult)
	}

	newAttempt := newUpstreamAttemptContext(context.Background())
	manager.MarkResult(newAttempt, Result{
		AuthID:   authID,
		Provider: "codex",
		Model:    model,
		Success:  false,
		Error: &Error{
			Code:       "upstream_error",
			Message:    "new request failed after recovery",
			HTTPStatus: 503,
		},
	})

	afterNewResult, ok := manager.GetByID(authID)
	if !ok || afterNewResult == nil || afterNewResult.ModelStates[model] == nil {
		t.Fatalf("new result was not recorded: %#v", afterNewResult)
	}
}

func TestResetAuthStateDoesNotWaitForExistingPersistence(t *testing.T) {
	store := &countingStore{}
	manager := NewManager(store, nil, nil)
	const authID = "non-blocking-recovery-auth"
	if _, err := manager.Register(context.Background(), &Auth{
		ID:       authID,
		Provider: "codex",
		Status:   StatusActive,
		Metadata: map[string]any{"type": "codex"},
	}); err != nil {
		t.Fatalf("register auth: %v", err)
	}

	lockVal, _ := manager.persistLocks.LoadOrStore(authID, &authPersistLock{})
	persistLock := lockVal.(*authPersistLock)
	persistLock.mu.Lock()

	done := make(chan error, 1)
	go func() {
		_, err := manager.ResetAuthState(context.Background(), authID)
		done <- err
	}()

	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("reset auth state: %v", err)
		}
	case <-time.After(250 * time.Millisecond):
		persistLock.mu.Unlock()
		t.Fatal("reset auth state waited for the persistence lock")
	}
	persistLock.mu.Unlock()
}
