package main

import (
	"context"
	"encoding/json"
	"github.com/gin-gonic/gin"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func continuityManifestFixture(t *testing.T) *manifest {
	t.Helper()
	m := &manifest{Accounts: []accountSpec{{ID: "local-a", AuthID: "sdk-a", ContinuityOwnerID: "owner-a"}, {ID: "local-b", AuthID: "sdk-b", ContinuityOwnerID: "owner-b"}}}
	m.accountByID = map[string]*accountSpec{}
	for i := range m.Accounts {
		m.accountByID[m.Accounts[i].ID] = &m.Accounts[i]
	}
	m.authManager = coreauth.NewManager(nil, nil, nil)
	for _, id := range []string{"a", "b"} {
		_, err := m.authManager.Register(context.Background(), &coreauth.Auth{ID: "sdk-" + id, Provider: "codex", Attributes: map[string]string{"account_id": "local-" + id}})
		if err != nil {
			t.Fatal(err)
		}
	}
	return m
}

func TestContinuityClientTrustedScopeOwnerAndOriginalHeaders(t *testing.T) {
	m := continuityManifestFixture(t)
	var mu sync.Mutex
	var firstID string
	var prepares int
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer "+strings.Repeat("f", 64) {
			t.Error("missing private control authentication")
		}
		defer r.Body.Close()
		switch r.URL.Path {
		case "/v1/continuity/prepare":
			var req coreexecutor.ContinuityPrepareRequest
			if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
				t.Error(err)
			}
			if req.Caller != "native:primary" || req.Headers.Get("Session-Id") != "client-session" || req.Headers.Get("Thread-Id") != "child" {
				t.Error("lost caller or original client identity", req)
			}
			mu.Lock()
			prepares++
			if firstID == "" {
				firstID = req.RequestID
			}
			if prepares > 1 && firstID == req.RequestID {
				t.Error("ordinary relay retry reused a released ticket")
			}
			mu.Unlock()
			_ = json.NewEncoder(w).Encode(map[string]any{"ticket": "ticket", "preferred_account_id": "local-b", "self_contained": true, "body": json.RawMessage(req.Body), "headers": map[string][]string{"session-id": {"canonical-client-session"}}})
		case "/v1/continuity/bind":
			var req coreexecutor.ContinuityBindRequest
			_ = json.NewDecoder(r.Body).Decode(&req)
			if req.AccountID != "local-b" || req.OwnerID != "owner-b" || req.Headers.Get("Authorization") != "Bearer fresh-upstream" {
				t.Error("owner or fresh credential mapping failed", req)
			}
			_ = json.NewEncoder(w).Encode(map[string]any{"ticket": req.Ticket, "attempt_id": req.AttemptID, "body": req.Body, "headers": map[string][]string{"session-id": {"mapped-b"}, "thread-id": {"mapped-child-b"}, "authorization": {"Bearer fresh-upstream"}}})
		default:
			t.Errorf("unexpected method %s", r.URL.Path)
			w.WriteHeader(404)
		}
	}))
	defer server.Close()
	client, err := newContinuityClient(server.URL, strings.Repeat("f", 64), m)
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	m.continuityClient = client
	ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{ID: "legacy", Enabled: true})
	ctx = withRequestContinuity(ctx, m, http.Header{"Session-Id": {"client-session"}, "Thread-Id": {"child"}}, false)
	for _, id := range []string{"relay-first", "relay-retry"} {
		prepared, err := client.Prepare(ctx, coreexecutor.ContinuityPrepareRequest{RequestID: id, Caller: "untrusted", Headers: http.Header{"Session-Id": {"wrong"}}, Body: json.RawMessage(`{"input":[]}`)})
		if err != nil || prepared.PreferredAccountID != "sdk-b" || prepared.Headers.Get("Session-Id") != "canonical-client-session" {
			t.Fatal("prepare mapping failed", prepared, err)
		}
	}
	bound, err := client.Bind(ctx, coreexecutor.ContinuityBindRequest{Ticket: "ticket", AttemptID: "attempt", AccountID: "sdk-b", OwnerID: "forged", Body: json.RawMessage(`{"input":[]}`), Headers: http.Header{"Authorization": {"Bearer fresh-upstream"}}})
	if err != nil || bound.Headers.Get("Session-Id") != "mapped-b" || bound.Headers.Get("Authorization") != "Bearer fresh-upstream" {
		t.Fatal("bind mapping failed", bound, err)
	}
}

func TestContinuityClientStateDependentHistoryRequiresAvailableOriginal(t *testing.T) {
	finishes := 0
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/release") {
			_, _ = io.WriteString(w, `{}`)
			return
		}
		if strings.HasSuffix(r.URL.Path, "/prepare") {
			_ = json.NewEncoder(w).Encode(map[string]any{"ticket": "reserved", "preferred_account_id": "removed-a", "self_contained": false, "body": map[string]any{}, "headers": map[string]any{}})
			return
		}
		var req coreexecutor.ContinuityFinishRequest
		_ = json.NewDecoder(r.Body).Decode(&req)
		if req.Ticket != "reserved" || req.AttemptID != "" || req.Status != "failed" {
			t.Error("invalid pre-bind failure", req)
		}
		finishes++
		_, _ = io.WriteString(w, `{}`)
	}))
	defer server.Close()
	client, _ := newContinuityClient(server.URL, strings.Repeat("a", 64), continuityManifestFixture(t))
	defer client.Close()
	ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{ID: "legacy", Enabled: true})
	result, err := client.Prepare(ctx, coreexecutor.ContinuityPrepareRequest{RequestID: "one", Body: json.RawMessage(`{"previous_response_id":"remote-only","input":[]}`)})
	if err == nil || result.PreferredAccountID != "removed-a" || finishes != 1 {
		t.Fatal("state-dependent history lost its original account binding", result, err, finishes)
	}
}

func TestContinuityClientAcceptsUnknownCompleteHistoryAndRemovedOriginal(t *testing.T) {
	body := json.RawMessage(`{"input":[{"role":"user","content":"old question"},{"type":"reasoning","id":"old-reasoning","encrypted_content":"unregistered-old-cipher"},{"type":"function_call","id":"old-tool","call_id":"old-call","name":"spawn_agent","namespace":"collaboration","arguments":"{\"message\":\"unregistered-agent-cipher\"}"},{"type":"function_call_output","call_id":"old-call","output":"old result"},{"role":"user","content":"continue"}]}`)
	for _, owner := range []string{"", "removed-a"} {
		t.Run("preferred="+owner, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				var req coreexecutor.ContinuityPrepareRequest
				if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
					t.Error(err)
				}
				if string(req.Body) != string(body) {
					t.Errorf("old history changed before prepare: %s", req.Body)
				}
				_ = json.NewEncoder(w).Encode(map[string]any{"ticket": "history", "preferred_account_id": owner, "self_contained": true, "body": req.Body, "headers": map[string]any{}})
			}))
			defer server.Close()
			client, _ := newContinuityClient(server.URL, strings.Repeat("a", 64), continuityManifestFixture(t))
			defer client.Close()
			ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{ID: "scoped", Enabled: true})
			result, err := client.Prepare(ctx, coreexecutor.ContinuityPrepareRequest{RequestID: "history", Body: body, AllowRebuild: true})
			if err != nil || result.PreferredAccountID != "" || !result.SelfContained || string(result.Body) != string(body) {
				t.Fatal("complete history could not select an eligible account", result, err)
			}
		})
	}
}

func TestContinuityClientControlErrorsDoNotCooldownOrLeakCredentials(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(409)
		_, _ = io.WriteString(w, `{"error":{"code":"account_binding_conflict","message":"Conflicting known account ownership"}}`)
	}))
	defer server.Close()
	client, _ := newContinuityClient(server.URL, strings.Repeat("a", 64), &manifest{})
	defer client.Close()
	err := client.call(context.Background(), "bind", map[string]string{"ticket": "one"}, nil)
	classified, ok := err.(continuityControlError)
	if !ok || classified.StatusCode() != 409 || !classified.IsRequestScoped() || strings.Contains(err.Error(), strings.Repeat("a", 64)) {
		t.Fatal("lost control error classification", err)
	}
	for _, endpoint := range []string{"https://127.0.0.1:1", "http://localhost:1", "http://vendor.invalid:1", "http://127.0.0.1:1/path", "http://user@127.0.0.1:1", "http://127.0.0.1:1?key=secret"} {
		if _, err := newContinuityClient(endpoint, strings.Repeat("a", 64), nil); err == nil {
			t.Error("accepted nonprivate endpoint", endpoint)
		}
	}
}

func TestContinuityClientRetryPlansAreIndependentAndWebsocketTurnsUseFreshHeaders(t *testing.T) {
	for _, websocket := range []bool{false, true} {
		t.Run(map[bool]string{false: "http-retry", true: "websocket-turn"}[websocket], func(t *testing.T) {
			var ids []string
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				var req coreexecutor.ContinuityPrepareRequest
				_ = json.NewDecoder(r.Body).Decode(&req)
				ids = append(ids, req.RequestID)
				expected := "connection-turn"
				if websocket {
					expected = "current-turn"
				}
				if req.Headers.Get("X-Codex-Turn-State") != expected || req.Headers.Get("Session-Id") != "connection-session" {
					t.Error("incorrect per-turn header precedence", req.Headers)
				}
				_ = json.NewEncoder(w).Encode(map[string]any{"ticket": "ordinary", "preferred_account_id": "local-a", "self_contained": true, "body": req.Body, "headers": map[string]any{}})
			}))
			defer server.Close()
			m := continuityManifestFixture(t)
			client, _ := newContinuityClient(server.URL, strings.Repeat("f", 64), m)
			m.continuityClient = client
			defer client.Close()
			ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{ID: "legacy", Enabled: true})
			ctx = withRequestContinuity(ctx, m, http.Header{"Session-Id": {"connection-session"}, "X-Codex-Turn-State": {"connection-turn"}}, websocket)
			for _, id := range []string{"first-id", "second-id"} {
				_, err := client.Prepare(ctx, coreexecutor.ContinuityPrepareRequest{RequestID: id, Body: json.RawMessage(`{"input":[]}`), Headers: http.Header{"X-Codex-Turn-State": {"current-turn"}}})
				if err != nil {
					t.Fatal(err)
				}
			}
			if len(ids) != 2 || ids[0] == ids[1] {
				t.Fatal("incorrect logical request identity", ids)
			}
		})
	}
}

func TestContinuityClientDoesNotFollowRedirectsOrEscapeHTMLPayloads(t *testing.T) {
	reached := false
	destination := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { reached = true }))
	defer destination.Close()
	redirect := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, destination.URL, http.StatusTemporaryRedirect)
	}))
	defer redirect.Close()
	client, _ := newContinuityClient(redirect.URL, strings.Repeat("f", 64), nil)
	defer client.Close()
	if err := client.call(context.Background(), "release", map[string]string{"ticket": "one"}, nil); err == nil || reached {
		t.Fatal("control plane followed redirect", err, reached)
	}
	payload := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		if strings.Contains(string(body), `\u003c`) || !strings.Contains(string(body), "<svg>&") {
			t.Error("control encoding expanded HTML payload", string(body))
		}
		_, _ = io.WriteString(w, `{}`)
	}))
	defer payload.Close()
	htmlClient, _ := newContinuityClient(payload.URL, strings.Repeat("f", 64), nil)
	defer htmlClient.Close()
	if err := htmlClient.call(context.Background(), "observe", map[string]json.RawMessage{"events": json.RawMessage(`["<svg>&"]`)}, nil); err != nil {
		t.Fatal(err)
	}
}

func TestContinuityClientErrorsReachClientWithoutDuplicateWrapping(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	ctx, _ := gin.CreateTestContext(recorder)
	ctx.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil)
	(&relayServer{}).writeExecutorError(ctx, continuityControlError{status: 409, code: "account_binding_conflict", message: "Conflicting known account ownership"})
	var result struct {
		Error struct {
			Code, Message string
		}
	}
	if json.Unmarshal(recorder.Body.Bytes(), &result) != nil || recorder.Code != 409 || result.Error.Code != "account_binding_conflict" || result.Error.Message != "Conflicting known account ownership" {
		t.Fatal("client error was wrapped or lost", recorder.Body.String())
	}
}

func TestContinuityClientStandaloneOperationsDoNotClaimTextHistory(t *testing.T) {
	calls := 0
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls++
		t.Error("standalone operation reached text continuity control plane")
	}))
	defer server.Close()
	client, _ := newContinuityClient(server.URL, strings.Repeat("a", 64), nil)
	defer client.Close()
	for _, kind := range []string{"image_generation", "image_edit", "audio", "other"} {
		ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{ID: "legacy", Enabled: true})
		ctx = context.WithValue(ctx, requestKindContextKey, kind)
		prepared, err := client.Prepare(ctx, coreexecutor.ContinuityPrepareRequest{RequestID: "standalone", Headers: http.Header{"Thread-Id": {"unrelated-text-thread"}}, Body: json.RawMessage(`{"model":"gpt-test","input":[{"role":"user","content":"generate an image"}]}`)})
		if err != nil || prepared.Ticket != "" {
			t.Fatal("standalone request claimed a conversation", kind, prepared, err)
		}
	}
	if calls != 0 {
		t.Fatal("text conversation may have been claimed", calls)
	}
}
