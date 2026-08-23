package auth

import (
	"context"
	"errors"
	"net/http"
	"testing"
	"time"
)

func TestMarkResultImageGenerationUnavailableSkipsModelSuspension(t *testing.T) {
	t.Parallel()

	manager := NewManager(nil, &RoundRobinSelector{}, nil)
	auth := &Auth{ID: "codex-sub2api", Provider: "codex", Metadata: map[string]any{"type": "codex"}}
	if _, err := manager.Register(context.Background(), auth); err != nil {
		t.Fatalf("register auth: %v", err)
	}

	manager.MarkResult(context.Background(), Result{
		AuthID:   auth.ID,
		Provider: auth.Provider,
		Model:    "gpt-5.4",
		Success:  false,
		Error: &Error{
			Message:    `{"error":{"code":"image_generation_not_enabled","message":"Image generation is not enabled for this group","type":"permission_error"}}`,
			HTTPStatus: http.StatusForbidden,
		},
	})

	updated, ok := manager.auths[auth.ID]
	if !ok || updated == nil {
		t.Fatalf("auth %q not found after MarkResult", auth.ID)
	}
	if !updated.ImageGenerationUnavailable {
		t.Fatalf("ImageGenerationUnavailable = false, want true")
	}
	if updated.Unavailable {
		t.Fatalf("auth.Unavailable = true, want false (text requests must stay allowed)")
	}
	if state := updated.ModelStates["gpt-5.4"]; state != nil && !state.NextRetryAfter.IsZero() {
		t.Fatalf("ModelState.NextRetryAfter = %v, want zero (no model suspension)", state.NextRetryAfter)
	}
}

func TestMarkResultOther403sKeepLegacyBehavior(t *testing.T) {
	t.Parallel()

	cases := []struct {
		name    string
		message string
	}{
		{name: "plain forbidden", message: `{"error":{"code":"auth_failed","message":"forbidden"}}`},
		{name: "unrelated permission error", message: `{"error":{"message":"Search is not enabled for this group","type":"permission_error"}}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			manager := NewManager(nil, &RoundRobinSelector{}, nil)
			auth := &Auth{ID: "codex-plain-" + tc.name, Provider: "codex", Metadata: map[string]any{"type": "codex"}}
			if _, err := manager.Register(context.Background(), auth); err != nil {
				t.Fatalf("register auth: %v", err)
			}
			manager.MarkResult(context.Background(), Result{
				AuthID:   auth.ID,
				Provider: auth.Provider,
				Model:    "gpt-5.4",
				Success:  false,
				Error:    &Error{Message: tc.message, HTTPStatus: http.StatusForbidden},
			})
			updated, ok := manager.auths[auth.ID]
			if !ok || updated == nil {
				t.Fatalf("auth %q not found after MarkResult", auth.ID)
			}
			if updated.ImageGenerationUnavailable {
				t.Fatalf("ImageGenerationUnavailable = true, want false")
			}
			state := updated.ModelStates["gpt-5.4"]
			if state == nil || state.NextRetryAfter.IsZero() {
				t.Fatalf("ModelState.NextRetryAfter = zero, want ~30m suspension preserved")
			}
		})
	}
}

func TestResultErrorFromErrorSetsImageCapabilityCode(t *testing.T) {
	err := resultErrorFromError(errors.New(`{"error":{"code":"image_generation_not_enabled","message":"Image generation is not enabled for this group","type":"permission_error"}}`))
	if err == nil || err.Code != "image_generation_not_enabled" {
		t.Fatalf("result error code = %q, want image_generation_not_enabled", err.Code)
	}
}

func TestShouldRetryAfterErrorImageCapabilityRetriesOnce(t *testing.T) {
	var m *Manager
	err := errors.New(`{"error":{"code":"image_generation_not_enabled","message":"Image generation is not enabled for this group","type":"permission_error"}}`)

	wait, ok := m.shouldRetryAfterError(err, 0, []string{"codex"}, "gpt-5.4", time.Minute)
	if !ok || wait != 0 {
		t.Fatalf("attempt 0: ok=%v wait=%v, want ok=true wait=0", ok, wait)
	}
	if _, ok := m.shouldRetryAfterError(err, 1, []string{"codex"}, "gpt-5.4", time.Minute); ok {
		t.Fatal("attempt 1 should not retry")
	}
}

func TestMarkResultSuccessClearsImageUnavailableForImageRequest(t *testing.T) {
	manager := NewManager(nil, &RoundRobinSelector{}, nil)

	imageAuth := &Auth{ID: "codex-recover", Provider: "codex", Metadata: map[string]any{"type": "codex"}, ImageGenerationUnavailable: true}
	if _, err := manager.Register(context.Background(), imageAuth); err != nil {
		t.Fatalf("register auth: %v", err)
	}
	manager.MarkResult(WithImageRequest(context.Background()), Result{
		AuthID: imageAuth.ID, Provider: imageAuth.Provider, Model: "gpt-image-2", Success: true,
	})
	if manager.auths[imageAuth.ID].ImageGenerationUnavailable {
		t.Fatal("ImageGenerationUnavailable = true after image success, want false")
	}

	textAuth := &Auth{ID: "codex-no-recover", Provider: "codex", Metadata: map[string]any{"type": "codex"}, ImageGenerationUnavailable: true}
	if _, err := manager.Register(context.Background(), textAuth); err != nil {
		t.Fatalf("register auth: %v", err)
	}
	manager.MarkResult(context.Background(), Result{
		AuthID: textAuth.ID, Provider: textAuth.Provider, Model: "gpt-5.4", Success: true,
	})
	if !manager.auths[textAuth.ID].ImageGenerationUnavailable {
		t.Fatal("plain text success should not clear the mark")
	}
}
