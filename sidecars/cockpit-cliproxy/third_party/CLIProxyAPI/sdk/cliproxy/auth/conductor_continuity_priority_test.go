package auth

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"reflect"
	"testing"

	internalconfig "github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executionregistry"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

type continuityPriorityLocalError struct {
	status int
	code   string
}

func (e *continuityPriorityLocalError) Error() string {
	body, _ := json.Marshal(map[string]any{"error": map[string]any{
		"code": e.code, "message": "Original local validation error", "param": "input",
	}})
	return string(body)
}
func (e *continuityPriorityLocalError) StatusCode() int       { return e.status }
func (e *continuityPriorityLocalError) IsRequestScoped() bool { return true }

type continuityPriorityUpstreamError struct {
	status  int
	body    string
	headers http.Header
}

func (e *continuityPriorityUpstreamError) Error() string        { return e.body }
func (e *continuityPriorityUpstreamError) StatusCode() int      { return e.status }
func (e *continuityPriorityUpstreamError) Headers() http.Header { return e.headers.Clone() }

func TestContinuityRetryErrorPreservesActualUpstreamAcrossManagerPaths(t *testing.T) {
	for _, home := range []bool{false, true} {
		for _, stream := range []bool{false, true} {
			for _, status := range []int{http.StatusTooManyRequests, http.StatusServiceUnavailable} {
				for _, localCode := range []string{"continuity_unverified_account_retry", "continuity_account_scope", "invalid_request_error"} {
					name := fmt.Sprintf("home=%t/stream=%t/status=%d/local=%s", home, stream, status, localCode)
					t.Run(name, func(t *testing.T) {
						provider, model := "codex", "continuity-priority-"+name
						original := &continuityPriorityUpstreamError{
							status:  status,
							body:    fmt.Sprintf(`{"error":{"code":"upstream_%d","message":"Original upstream failure","param":"reasoning.effort"}}`, status),
							headers: http.Header{"Retry-After": {"7"}, "X-Request-Id": {"original-upstream-request"}},
						}
						local := &continuityPriorityLocalError{status: http.StatusConflict, code: localCode}
						if localCode == "invalid_request_error" {
							local.status = http.StatusBadRequest
						}
						executor := &retryTerminalPriorityExecutor{identifier: provider, upstreamErr: original, terminalErr: local}
						manager := NewManager(nil, nil, nil)
						manager.SetRetryConfig(1, 0, 0)
						if home {
							provider = "home-retry-contract"
							executor.identifier = provider
							manager.SetConfig(&internalconfig.Config{Home: internalconfig.HomeConfig{Enabled: true}})
							manager.PublishHomeDispatch(&retryTerminalPriorityHomeDispatcher{}, executionregistry.New(), 1)
						} else {
							registerRetryRoundLocalAuths(t, manager, provider, model, map[string]int{"continuity-a": 0, "continuity-b": 0})
						}
						manager.RegisterExecutor(executor)
						var got error
						if stream {
							_, got = manager.ExecuteStream(context.Background(), []string{provider}, cliproxyexecutor.Request{Model: model}, cliproxyexecutor.Options{Stream: true})
						} else {
							_, got = manager.Execute(context.Background(), []string{provider}, cliproxyexecutor.Request{Model: model}, cliproxyexecutor.Options{})
						}
						if localCode == "continuity_unverified_account_retry" {
							var preserved *continuityPriorityUpstreamError
							if !errors.As(got, &preserved) || preserved != original || statusCodeFromError(got) != status || got.Error() != original.body {
								t.Fatalf("error=%T %v, want original upstream status/body/param", got, got)
							}
							if !reflect.DeepEqual(preserved.Headers(), original.headers) {
								t.Fatalf("headers=%v, want %v", preserved.Headers(), original.headers)
							}
						} else if !errors.Is(got, local) || errors.Is(got, original) {
							t.Fatalf("error=%v, want authoritative local request/scope error %v", got, local)
						}
						if calls := executor.calls.Load(); calls != 2 {
							t.Fatalf("executor calls=%d, want one upstream failure and one unsent candidate", calls)
						}
					})
				}
			}
		}
	}
}

func TestContinuityRetryWithoutActualUpstreamKeepsItsLocalError(t *testing.T) {
	local := &continuityPriorityLocalError{status: http.StatusConflict, code: "continuity_unverified_account_retry"}
	for _, earlier := range []error{nil, errors.New("earlier local preparation failure")} {
		if got := preserveContinuityUpstreamError(local, earlier); got != local || isRequestStopError(got) {
			t.Fatalf("error=%v, want original local rejection without a synthetic upstream", got)
		}
	}
	// The local rejection remains an unsent candidate for the Codex transport.
	ctx := newUpstreamAttemptContext(context.Background())
	attempted := map[string]struct{}{}
	recordCredentialAttempt(attempted, cliproxyexecutor.Options{}, "candidate-b", ctx, "codex", local)
	if len(attempted) != 0 || cliproxyexecutor.UpstreamAttempted(ctx) {
		t.Fatal("a locally rejected account retry was counted as an upstream send")
	}
}
