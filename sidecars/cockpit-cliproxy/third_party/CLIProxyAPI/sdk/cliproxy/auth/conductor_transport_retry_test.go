package auth

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"testing"
	"time"

	internalconfig "github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestTransportRetryRemainsBoundedAndRequestScoped(t *testing.T) {
	m := NewManager(nil, nil, nil)
	m.SetRetryConfig(1, 0, 0)
	registerRetryRoundLocalAuths(t, m, "codex", "transport-model", map[string]int{"transport-auth": 1})
	for _, tc := range []struct {
		name string
		err  error
		want bool
	}{
		{"tls-eof", errors.New("utls: TLS handshake: EOF"), true},
		{"eof", io.EOF, true},
		{"dns", &net.DNSError{Err: "no such host", Name: "example.invalid"}, true},
		{"reset", errors.New("connection reset by peer"), true},
		{"windows-reset", errors.New("forcibly closed by the remote host"), true},
		{"cancel", fmt.Errorf("request: %w", context.Canceled), false},
		{"deadline", context.DeadlineExceeded, false},
		{"request-scoped", &Error{Code: ErrorCodeRequestScoped, Message: "TLS handshake: EOF"}, false},
		{"stop", wrapRequestStopError(io.EOF), false},
		{"unauthorized", &Error{HTTPStatus: 401, Message: "TLS handshake: EOF"}, false},
		{"quota", &Error{HTTPStatus: 429, Message: "quota exceeded"}, true},
		{"unknown", errors.New("unclassified error"), false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			wait, retry := m.shouldRetryAfterError(tc.err, 0, []string{"codex"}, "transport-model", 0)
			if retry != tc.want || wait != 0 {
				t.Fatalf("retry = (%v, %v), want (0, %v)", wait, retry, tc.want)
			}
			if _, retry = m.shouldRetryAfterError(tc.err, 1, []string{"codex"}, "transport-model", 0); retry {
				t.Fatal("exceeded configured retry round")
			}
		})
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, retry := m.shouldRetryAfterErrorWithAttempted(ctx, cliproxyexecutor.Options{}, io.EOF, 0, []string{"codex"}, "transport-model", 0, -1, 1, nil); retry {
		t.Fatal("retried canceled request")
	}
	m.SetConfig(&internalconfig.Config{Home: internalconfig.HomeConfig{Enabled: true}})
	for attempt := 0; attempt < 2; attempt++ {
		err := markHomeRetryRoundExhausted(io.EOF, nil, true)
		_, retry := m.shouldRetryAfterErrorWithHomeRetryLimit(context.Background(), cliproxyexecutor.Options{}, err, attempt, []string{"codex"}, "transport-model", 0, 1, 1)
		if retry != (attempt == 0) {
			t.Fatalf("Home round %d retry = %v", attempt, retry)
		}
	}
}

func TestTriedExhaustionOnlySuppressesHealthyPoolDiagnostics(t *testing.T) {
	for _, mixed := range []bool{false, true} {
		for _, state := range []string{"healthy", "healthy-with-cooling-peer", "cooling", "disabled", "unrelated-tried", "wrong-model"} {
			t.Run(fmt.Sprintf("mixed=%v/%s", mixed, state), func(t *testing.T) {
				selector := &selectionFailureReportingSelector{Selector: &RoundRobinSelector{}}
				m := NewManager(nil, selector, nil)
				m.RegisterExecutor(schedulerTestExecutor{provider: "codex"})
				auth := &Auth{ID: "tried-auth", Provider: "codex", Status: StatusActive}
				if state == "cooling" {
					auth.Unavailable = true
					auth.NextRetryAfter = time.Now().Add(time.Hour)
				}
				if state == "disabled" {
					auth.Disabled = true
				}
				if _, err := m.Register(context.Background(), auth); err != nil {
					t.Fatal(err)
				}
				registerSchedulerModels(t, "codex", "transport-model", auth.ID)
				if state == "healthy-with-cooling-peer" {
					if _, err := m.Register(context.Background(), &Auth{ID: "cooling-peer", Provider: "codex", Unavailable: true, NextRetryAfter: time.Now().Add(time.Hour)}); err != nil {
						t.Fatal(err)
					}
					registerSchedulerModels(t, "codex", "transport-model", "cooling-peer")
				}
				tried := map[string]struct{}{auth.ID: {}}
				model := "transport-model"
				if state == "unrelated-tried" {
					tried = map[string]struct{}{"other-auth": {}}
					model = "unsupported-model"
				}
				if state == "wrong-model" {
					model = "unsupported-model"
				}
				var err error
				if mixed {
					_, _, _, err = m.pickNextMixedLegacy(context.Background(), []string{"codex"}, model, cliproxyexecutor.Options{}, tried)
				} else {
					_, _, err = m.pickNextLegacy(context.Background(), "codex", model, cliproxyexecutor.Options{}, tried)
				}
				if err == nil {
					t.Fatal("expected selection failure")
				}
				if selector.called != (state != "healthy" && state != "healthy-with-cooling-peer") {
					t.Fatalf("reporter called = %v for %s", selector.called, state)
				}
			})
		}
	}
}

type transientRetryExecutor struct {
	schedulerTestExecutor
	calls int
}

func (e *transientRetryExecutor) Execute(context.Context, *Auth, cliproxyexecutor.Request, cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	e.calls++
	return cliproxyexecutor.Response{}, errors.New("utls: TLS handshake: EOF")
}

func TestExecuteTransportFailureRetriesWithoutPoolOutage(t *testing.T) {
	selector := &selectionFailureReportingSelector{Selector: &RoundRobinSelector{}}
	m := NewManager(nil, selector, nil)
	m.SetRetryConfig(1, 0, 0)
	e := &transientRetryExecutor{schedulerTestExecutor: schedulerTestExecutor{provider: "codex"}}
	m.RegisterExecutor(e)
	if _, err := m.Register(context.Background(), &Auth{ID: "transport-auth", Provider: "codex", Status: StatusActive}); err != nil {
		t.Fatal(err)
	}
	registerSchedulerModels(t, "codex", "transport-model", "transport-auth")
	_, err := m.Execute(context.Background(), []string{"codex"}, cliproxyexecutor.Request{Model: "transport-model"}, cliproxyexecutor.Options{})
	if err == nil || err.Error() != "utls: TLS handshake: EOF" {
		t.Fatalf("unexpected terminal error: %v", err)
	}
	if e.calls != 2 {
		t.Fatalf("upstream attempts = %d, want 2", e.calls)
	}
	if selector.called {
		t.Fatal("transient failure reported as pool outage")
	}
	m.mu.RLock()
	defer m.mu.RUnlock()
	auth := m.auths["transport-auth"]
	if auth.Unavailable || !auth.NextRetryAfter.IsZero() || auth.Quota.Exceeded {
		t.Fatalf("transport failure cooled auth: %#v", auth)
	}
	for _, state := range auth.ModelStates {
		if state != nil && (state.Unavailable || !state.NextRetryAfter.IsZero() || state.Quota.Exceeded) {
			t.Fatalf("transport failure cooled model: %#v", state)
		}
	}
}
