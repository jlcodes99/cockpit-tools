package main

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestHTTPPoolDiagnosticsFollowFinalRequestOutcome(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, tc := range []struct {
		name, message    string
		status, wantPool int
	}{
		{"fallback completed", "", 200, 0},
		{"TLS failure after tried credentials", `Post "https://chatgpt.com/backend-api/codex/responses": utls: TLS handshake: EOF`, 502, 0},
		{"upstream mentions local code", "upstream_error: upstream reported auth_unavailable: disconnected", 502, 0},
		{"client canceled", "context canceled", 499, 0},
		{"quota guard rejected next tier", "no auth available: bound OAuth quota reserve blocked 1 auth(s)", 503, 0},
		{"final scope failure", "auth_unavailable: no eligible account", 503, 1},
		{"final structured scope failure", `{"error":{"code":"auth_unavailable","message":"no eligible account"}}`, 503, 1},
		{"selection failure in committed stream", `{"error":{"code":"auth_not_found","message":"no eligible account"}}`, 200, 1},
	} {
		t.Run(tc.name, func(t *testing.T) {
			bound := &accountSpec{ID: "bound"}
			outside := &accountSpec{ID: "outside"}
			key := &apiKeySpec{ID: "key", Key: "client-key", Enabled: true, AccountIDs: []string{"bound"}}
			m := &manifest{apiKeyByValue: map[string]*apiKeySpec{"client-key": key}, accountByID: map[string]*accountSpec{"bound": bound, "outside": outside}}
			emitter := &eventEmitter{}
			policy := &requestPolicy{manifest: m, emitter: emitter}
			selector := &cockpitSelector{manifest: m, emitter: emitter}
			router := gin.New()
			router.Use(policy.middleware())
			router.GET("/v1/responses", func(c *gin.Context) {
				ctx := c.Request.Context()
				// A preferred tier has no matching auth; fallback selects a bound
				// credential; after upstream failure only an outside auth remains.
				_, _ = selector.Pick(ctx, "mixed", "gpt-6.1-sol", coreexecutor.Options{}, []*coreauth.Auth{{ID: "outside"}})
				_, err := selector.Pick(ctx, "mixed", "gpt-6.1-sol", coreexecutor.Options{}, []*coreauth.Auth{{ID: "bound"}})
				if err != nil {
					t.Fatal(err)
				}
				_, _ = selector.Pick(ctx, "mixed", "gpt-6.1-sol", coreexecutor.Options{}, []*coreauth.Auth{{ID: "outside"}})
				if tc.message != "" {
					_ = c.Error(errors.New("auth_unavailable: earlier routing tier"))
					_ = c.Error(errors.New(tc.message))
				}
				c.Status(tc.status)
			})
			out := captureStdout(t, func() {
				req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
				req.Header.Set("Authorization", "Bearer client-key")
				router.ServeHTTP(httptest.NewRecorder(), req)
			})
			attempts, pools, completed := 0, 0, false
			for _, line := range strings.Split(strings.TrimSpace(out), "\n") {
				var event requestDiagnosticPayload
				if err := json.Unmarshal([]byte(line), &event); err != nil {
					t.Fatal(err)
				}
				switch event.Type {
				case "auth_pool_attempt":
					attempts++
				case "auth_pool_result":
					pools++
					if event.CandidateAuths != 1 || len(event.ScopeDiagnostics) != 2 {
						t.Fatalf("lost final scope detail: %+v", event)
					}
				case "request_completed":
					completed = true
				}
			}
			if attempts != 2 || pools != tc.wantPool || !completed {
				t.Fatalf("attempts=%d pools=%d completed=%t: %s", attempts, pools, completed, out)
			}
		})
	}
}

func TestHTTPPoolRecoveryRequiresFinalSuccessfulUsage(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, tc := range []struct{ success, record bool }{{false, true}, {true, true}, {false, false}} {
		success := tc.success
		ctx := withRequestPoolDiagnostics(context.Background(), "recovery")
		c, _ := gin.CreateTestContext(httptest.NewRecorder())
		c.Request = httptest.NewRequest("POST", "/v1/responses", nil).WithContext(ctx)
		policy := &requestPolicy{emitter: &eventEmitter{}, tracker: newRequestUsageTracker()}
		if tc.record {
			policy.tracker.record(usagePayload{RequestID: "recovery", Success: success, outputComplete: success})
		}
		out := captureStdout(t, func() {
			policy.emitRequestCompleted(c, "recovery", &apiKeySpec{ID: "key"}, "text", "gpt-6.1-sol", time.UnixMilli(100))
		})
		count := 0
		for _, line := range strings.Split(strings.TrimSpace(out), "\n") {
			var event requestDiagnosticPayload
			if err := json.Unmarshal([]byte(line), &event); err != nil {
				t.Fatal(err)
			}
			if event.Type == "auth_result" {
				count++
				if event.Success == nil || !*event.Success || event.AccountID != "" || event.Provider != "mixed" || event.APIKeyID != "key" || event.Model != "gpt-6.1-sol" || event.StartedAtMS != 100 {
					t.Fatalf("wrong recovery route: %+v", event)
				}
			}
		}
		want := 0
		if success {
			want = 1
		}
		if count != want {
			t.Fatalf("success=%t recovery events=%d: %s", success, count, out)
		}
	}
}

func TestHTTPPoolDiagnosticsPreserveFinalAdmissionPolicy(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, code := range []string{"account_paused", "quota_reserved"} {
		t.Run(code, func(t *testing.T) {
			base, auths := rotationSelectorForTest()
			base.emitter = &eventEmitter{}
			m := base.manifest
			key := &apiKeySpec{ID: "key", Key: "client-key", Enabled: true, AccountIDs: []string{"a"}}
			m.apiKeyByValue = map[string]*apiKeySpec{key.Key: key}
			policy := &requestPolicy{manifest: m, emitter: base.emitter}
			router := gin.New()
			router.Use(policy.middleware())
			router.POST("/v1/responses", func(c *gin.Context) {
				err := base.ReportAuthSelectionFailure(c.Request.Context(), "mixed", "gpt-5.4", auths, &coreauth.Error{Code: code, Message: "local admission rejected", HTTPStatus: 503})
				(&relayServer{}).writeExecutorError(c, err)
			})
			out := captureStdout(t, func() {
				req := httptest.NewRequest("POST", "/v1/responses", nil)
				req.Header.Set("Authorization", "Bearer client-key")
				router.ServeHTTP(httptest.NewRecorder(), req)
			})
			count := 0
			for _, line := range strings.Split(strings.TrimSpace(out), "\n") {
				var event requestDiagnosticPayload
				if err := json.Unmarshal([]byte(line), &event); err != nil {
					t.Fatal(err)
				}
				if event.Type == "auth_pool_result" {
					count++
					if event.ErrorCode != code {
						t.Fatalf("admission policy changed: %+v", event)
					}
				}
			}
			if count != 1 {
				t.Fatalf("final %s diagnostic missing or duplicated: %s", code, out)
			}
			if finalRequestPoolFailure(503, code+": rejected", "auth_unavailable") {
				t.Fatal("an earlier scope failure replaced the final admission policy")
			}
		})
	}
}

func TestDeferredPoolDiagnosticsAreRequestLocalAndFinishOnce(t *testing.T) {
	emitter := &eventEmitter{}
	a := withRequestPoolDiagnostics(context.Background(), "a")
	b := withRequestPoolDiagnostics(context.Background(), "b")
	out := captureStdout(t, func() {
		deferRequestPoolDiagnostic(a, emitter, requestDiagnosticPayload{Type: "auth_pool_result", RequestID: "a", APIKeyID: "key-a", ErrorCode: "auth_unavailable"})
		deferRequestPoolDiagnostic(b, emitter, requestDiagnosticPayload{Type: "auth_pool_result", RequestID: "b", APIKeyID: "key-b", ErrorCode: "auth_unavailable"})
		finishRequestPoolDiagnostics(a, emitter, 200, "")
		finishRequestPoolDiagnostics(b, emitter, 503, "auth_unavailable: no auth available")
		finishRequestPoolDiagnostics(b, emitter, 503, "auth_unavailable: no auth available")
		deferRequestPoolDiagnostic(a, emitter, requestDiagnosticPayload{Type: "auth_pool_result", RequestID: "a"})
		finishRequestPoolDiagnostics(a, emitter, 503, "auth_unavailable: late event")
	})
	var results []requestDiagnosticPayload
	for _, line := range strings.Split(strings.TrimSpace(out), "\n") {
		var event requestDiagnosticPayload
		if err := json.Unmarshal([]byte(line), &event); err != nil {
			t.Fatal(err)
		}
		if event.Type == "auth_pool_result" {
			results = append(results, event)
		}
	}
	if len(results) != 1 || results[0].RequestID != "b" || results[0].APIKeyID != "key-b" {
		t.Fatalf("request outcomes leaked or reopened: %+v", results)
	}
	for _, ctx := range []context.Context{a, b} {
		state := ctx.Value(requestPoolDiagnosticsKey{}).(*requestPoolDiagnostics)
		if !state.finished || state.pending != nil {
			t.Fatal("pending diagnostic retained after completion")
		}
	}
}
