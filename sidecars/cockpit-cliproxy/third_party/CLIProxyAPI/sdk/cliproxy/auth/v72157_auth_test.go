package auth

import (
	"context"
	"net/http"
	"testing"
	"time"
)

func TestV72157StructuredModelNotFoundUsesModelCooldown(t *testing.T) {
	previous := quotaCooldownDisabled.Load()
	quotaCooldownDisabled.Store(false)
	t.Cleanup(func() { quotaCooldownDisabled.Store(previous) })

	rawErr := &statusBearingError{
		status: http.StatusBadRequest,
		msg:    `{"error":{"type":"invalid_request_error","code":"model_not_found","message":"The model gpt-test does not exist or you do not have access to it."}}`,
	}
	if isRequestInvalidError(rawErr) {
		t.Fatal("model_not_found was classified as a caller request fault")
	}
	resultErr := resultErrorFromError(rawErr)
	if resultErr == nil || resultErr.Code != "model_not_found" {
		t.Fatalf("result error = %#v", resultErr)
	}

	m := NewManager(nil, nil, nil)
	auth := &Auth{ID: "codex-model-not-found", Provider: "codex"}
	if _, err := m.Register(context.Background(), auth); err != nil {
		t.Fatalf("register: %v", err)
	}
	m.MarkResult(context.Background(), Result{AuthID: auth.ID, Provider: auth.Provider, Model: "gpt-test", Error: resultErr})
	updated, ok := m.GetByID(auth.ID)
	if !ok || updated == nil || updated.ModelStates["gpt-test"] == nil {
		t.Fatalf("missing model cooldown state: %#v", updated)
	}
	remaining := time.Until(updated.ModelStates["gpt-test"].NextRetryAfter)
	if remaining < 11*time.Hour || remaining > 13*time.Hour {
		t.Fatalf("cooldown remaining = %v, want about 12h", remaining)
	}
}

func TestV72157PermanentUnauthorizedSelectionIsTerminal(t *testing.T) {
	m := NewManager(nil, nil, nil)
	auth := &Auth{
		ID:          "revoked-codex",
		Provider:    "codex",
		Status:      StatusError,
		Unavailable: true,
		UpdatedAt:   time.Now(),
		LastError: &Error{
			Code:       "unauthorized",
			Message:    "refresh token revoked",
			Retryable:  false,
			HTTPStatus: http.StatusUnauthorized,
		},
	}
	_, err := m.availableAuthsForRouteModelWithPriorityMode([]*Auth{auth}, "codex", "gpt-test", time.Now(), false)
	if err == nil || !IsTerminalAuthError(err) {
		t.Fatalf("error = %v, want terminal auth error", err)
	}
}
