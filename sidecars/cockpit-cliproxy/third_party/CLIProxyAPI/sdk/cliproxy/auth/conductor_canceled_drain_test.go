package auth

import (
	"context"
	api "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"testing"
	"time"
)

type canceledDrainHook struct {
	resultCaptureHook
	observed chan Result
}

func (h *canceledDrainHook) OnResult(ctx context.Context, r Result) {
	h.resultCaptureHook.OnResult(ctx, r)
	h.observed <- r
}
func awaitCanceledDrainResult(t *testing.T, h *canceledDrainHook) Result {
	t.Helper()
	select {
	case r := <-h.observed:
		return r
	case <-time.After(time.Second):
		t.Fatal("canceled drain lost result")
		return Result{}
	}
}
func closeCanceledDrainOutput(t *testing.T, ch <-chan api.StreamChunk) {
	t.Helper()
	select {
	case chunk, ok := <-ch:
		if ok {
			t.Fatalf("canceled stream emitted %#v", chunk)
		}
	case <-time.After(time.Second):
		t.Fatal("client output waited for upstream drain")
	}
}

func TestCanceledStreamDrainsConfirmedQuotaAfterPayload(t *testing.T) {
	for _, place := range []string{"buffered", "remaining", "empty", "cancel_error"} {
		t.Run(place, func(t *testing.T) {
			h := &canceledDrainHook{observed: make(chan Result, 4)}
			m := NewManager(nil, &RoundRobinSelector{}, h)
			auth, err := m.Register(t.Context(), &Auth{ID: "canceled-drain", Provider: "codex", Status: StatusActive})
			if err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithCancel(t.Context())
			cancel()
			source := make(chan api.StreamChunk, 4)
			prefix := api.StreamChunk{Payload: []byte("first")}
			var buffered []api.StreamChunk
			switch place {
			case "buffered":
				buffered = []api.StreamChunk{prefix}
			case "remaining":
				source <- prefix
			case "empty":
				source <- api.StreamChunk{}
			case "cancel_error":
				source <- api.StreamChunk{Err: context.Canceled}
			}
			source <- api.StreamChunk{Err: issue6503QuotaError{retryAfter: time.Hour}}
			source <- api.StreamChunk{Err: issue6503QuotaError{retryAfter: time.Hour}}
			close(source)
			opts := api.Options{Metadata: map[string]any{"caller_scope": "original-scope"}}
			stream := m.wrapStreamResult(ctx, auth, "codex", "test", "test", nil, buffered, source, OAuthModelAliasResult{}, false, opts)
			closeCanceledDrainOutput(t, stream.Chunks)
			result := awaitCanceledDrainResult(t, h)
			if result.Success || !result.CredentialScope || result.Options.Metadata["caller_scope"] != "original-scope" {
				t.Fatalf("scope lost: %#v", result)
			}
			deadline := time.Now().Add(time.Second)
			for len(source) > 0 && time.Now().Before(deadline) {
				time.Sleep(time.Millisecond)
			}
			latest, _ := m.GetByID(auth.ID)
			if !latest.Unavailable || !latest.NextRetryAfter.After(time.Now().Add(30*time.Minute)) || latest.Failed != 1 || latest.Success != 0 {
				t.Fatalf("quota not recorded once: %#v", latest)
			}
			if len(h.Results()) != 1 {
				t.Fatalf("duplicate failure results: %#v", h.Results())
			}
		})
	}
}

func TestCanceledStreamDrainKeepsCredentialFence(t *testing.T) {
	for _, registration := range []bool{false, true} {
		h := &canceledDrainHook{observed: make(chan Result, 4)}
		m := NewManager(nil, &RoundRobinSelector{}, h)
		old, err := m.Register(t.Context(), &Auth{ID: "drain-fence", Provider: "codex", Metadata: map[string]any{"access_token": "old"}})
		if err != nil {
			t.Fatal(err)
		}
		replacement := old.Clone()
		replacement.Metadata["access_token"] = "new"
		if registration {
			_, err = m.Register(t.Context(), replacement)
		} else {
			_, err = m.Update(t.Context(), replacement)
		}
		if err != nil {
			t.Fatal(err)
		}
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		source := make(chan api.StreamChunk, 1)
		stream := m.wrapStreamResult(ctx, old, "codex", "test", "test", nil, []api.StreamChunk{{Payload: []byte("first")}}, source, OAuthModelAliasResult{}, false, api.Options{})
		closeCanceledDrainOutput(t, stream.Chunks) // source is still open and has no error yet.
		source <- api.StreamChunk{Err: issue6503QuotaError{retryAfter: time.Hour}}
		close(source)
		result := awaitCanceledDrainResult(t, h)
		if !result.StaleCredential || !result.CredentialScope || result.CredentialVersion != old.CredentialVersion || result.RegistrationEpoch != old.RegistrationEpoch {
			t.Fatalf("fence lost: %#v", result)
		}
		latest, _ := m.GetByID(old.ID)
		if latest.Unavailable || latest.LastError != nil || latest.Failed != 0 || latest.Success != 0 {
			t.Fatalf("old quota changed replacement: %#v", latest)
		}
	}
}

func TestCanceledStreamDoesNotRecordFalseSuccessOrFault(t *testing.T) {
	for _, withError := range []bool{false, true} {
		h := &canceledDrainHook{observed: make(chan Result, 4)}
		m := NewManager(nil, &RoundRobinSelector{}, h)
		auth, err := m.Register(t.Context(), &Auth{ID: "drain-pure-cancel", Provider: "codex", Status: StatusActive})
		if err != nil {
			t.Fatal(err)
		}
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		source := make(chan api.StreamChunk, 2)
		source <- api.StreamChunk{}
		if withError {
			source <- api.StreamChunk{Err: context.Canceled}
		}
		close(source)
		stream := m.wrapStreamResult(ctx, auth, "codex", "test", "test", nil, nil, source, OAuthModelAliasResult{}, false, api.Options{})
		closeCanceledDrainOutput(t, stream.Chunks)
		deadline := time.Now().Add(time.Second)
		for len(source) > 0 && time.Now().Before(deadline) {
			time.Sleep(time.Millisecond)
		}
		latest, _ := m.GetByID(auth.ID)
		if latest.Unavailable || latest.LastError != nil || latest.Failed != 0 || latest.Success != 0 || len(h.Results()) != 0 {
			t.Fatalf("cancellation mutated auth: %#v results=%#v", latest, h.Results())
		}
	}
}

func TestCanceledStreamDrainKeepsModelAndHomeScopes(t *testing.T) {
	for _, ephemeral := range []bool{false, true} {
		h := &canceledDrainHook{observed: make(chan Result, 4)}
		m := NewManager(nil, &RoundRobinSelector{}, h)
		auth, err := m.Register(t.Context(), &Auth{ID: "drain-scope", Provider: "codex", Status: StatusActive})
		if err != nil {
			t.Fatal(err)
		}
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		source := make(chan api.StreamChunk, 1)
		source <- api.StreamChunk{Err: &retryAfterStatusError{status: 429, message: "rate limited", retryAfter: time.Hour}}
		close(source)
		stream := m.wrapStreamResult(ctx, auth, "codex", "test", "test", nil, []api.StreamChunk{{Payload: []byte("first")}}, source, OAuthModelAliasResult{}, ephemeral, api.Options{})
		closeCanceledDrainOutput(t, stream.Chunks)
		result := awaitCanceledDrainResult(t, h)
		if result.CredentialScope || result.Success || result.CredentialVersion != auth.CredentialVersion || result.RegistrationEpoch != auth.RegistrationEpoch {
			t.Fatalf("model-scope result changed: %#v", result)
		}
		latest, _ := m.GetByID(auth.ID)
		if ephemeral {
			if latest.Unavailable || latest.Failed != 0 || len(latest.ModelStates) > 0 {
				t.Fatalf("Home result mutated local auth: %#v", latest)
			}
		} else {
			state := latest.ModelStates["test"]
			if state == nil || !state.Unavailable || !state.NextRetryAfter.After(time.Now().Add(30*time.Minute)) {
				t.Fatalf("model-scoped cooldown missing: %#v", state)
			}
			if latest.Quota.Reason == "credential_quota" {
				t.Fatal("model error became global credential quota")
			}
		}
	}
}
