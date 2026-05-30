package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func TestCodexClientModelsResponseShape(t *testing.T) {
	response := buildCodexClientModelsResponse([]string{"gpt-5.4", "gpt-image-2", codexAutoReviewModel})
	models, ok := response["models"].([]gin.H)
	if !ok {
		t.Fatalf("models response should contain a models array: %#v", response["models"])
	}
	if len(models) != 3 {
		t.Fatalf("expected 3 models, got %d", len(models))
	}
	if models[0]["slug"] != "gpt-5.4" || models[0]["prefer_websockets"] != true {
		t.Fatalf("unexpected first model: %#v", models[0])
	}
	if models[1]["visibility"] != "hide" {
		t.Fatalf("image model should be hidden in Codex client catalog: %#v", models[1])
	}
	if models[2]["slug"] != codexAutoReviewModel || models[2]["visibility"] != "hide" {
		t.Fatalf("auto review model should be hidden in Codex client catalog: %#v", models[2])
	}
}

func TestVisibleModelsForAPIKeyUsesPrefixAndFilters(t *testing.T) {
	spec := &apiKeySpec{
		ModelPrefix:    "team",
		AllowedModels:  []string{"gpt-*"},
		ExcludedModels: []string{"gpt-image-*"},
	}
	m := &manifest{
		ModelIDs: []string{"gpt-5.4", "gpt-image-2", "custom-model"},
	}

	models := visibleModelsForAPIKey(m, spec)

	if len(models) != 1 || models[0] != "team/gpt-5.4" {
		t.Fatalf("unexpected visible models: %#v", models)
	}
}

func TestClientCatalogModelsIncludesAutoReviewWithoutPrefix(t *testing.T) {
	spec := &apiKeySpec{
		ModelPrefix:    "team",
		AllowedModels:  []string{"gpt-*"},
		ExcludedModels: []string{"gpt-image-*"},
	}
	m := &manifest{
		ModelIDs: []string{"gpt-5.4", "gpt-image-2", "custom-model"},
	}

	models := clientCatalogModelsForAPIKey(m, spec)

	if len(models) != 2 || models[0] != "team/gpt-5.4" || models[1] != codexAutoReviewModel {
		t.Fatalf("unexpected client catalog models: %#v", models)
	}
}

func TestCanonicalModelForClientModelHandlesPrefixAliasAndSnapshot(t *testing.T) {
	spec := &apiKeySpec{ModelPrefix: "team"}
	m := &manifest{
		ModelIDs:      []string{"gpt-5.4", "gpt-5.4-mini"},
		aliasToSource: map[string]string{"fast": "gpt-5.4-mini"},
	}

	if got := canonicalModelForClientModel(m, spec, "team/fast"); got != "gpt-5.4-mini" {
		t.Fatalf("alias should resolve to source model, got %q", got)
	}
	if got := canonicalModelForClientModel(m, spec, "team/gpt-5.4-2026-03-05"); got != "gpt-5.4" {
		t.Fatalf("snapshot should resolve to supported model, got %q", got)
	}
	if got := canonicalModelForClientModel(m, spec, codexAutoReviewModel); got != codexAutoReviewModel {
		t.Fatalf("auto review model should stay canonical, got %q", got)
	}
}

func TestLoadManifestIndexesAPIKeyAccounts(t *testing.T) {
	path := filepath.Join(t.TempDir(), "manifest.json")
	if err := os.WriteFile(path, []byte(`{
		"apiKeys": [{"id":"client","label":"Client","key":"client-key","enabled":true}],
		"accounts": [{"id":"api-account","email":"api@example.com","upstreamApiKey":"  sk-upstream  "}]
	}`), 0o644); err != nil {
		t.Fatalf("write manifest: %v", err)
	}

	m, err := loadManifest(path)
	if err != nil {
		t.Fatalf("load manifest: %v", err)
	}

	account := m.accountByAPIKey["sk-upstream"]
	if account == nil {
		t.Fatalf("API Key account should be indexed by upstream key: %#v", m.accountByAPIKey)
	}
	if account.ID != "api-account" || account.UpstreamAPIKey != "sk-upstream" {
		t.Fatalf("unexpected indexed account: %#v", account)
	}
}

func TestSidecarRuntimeRegistersConfigCodexAPIKeyAuths(t *testing.T) {
	tempDir := t.TempDir()
	authDir := filepath.Join(tempDir, "auths")
	configPath := filepath.Join(tempDir, "config.json")
	if err := os.WriteFile(configPath, []byte(`{}`), 0o644); err != nil {
		t.Fatalf("write config path: %v", err)
	}

	cfg := &config.Config{
		AuthDir: authDir,
		CodexKey: []config.CodexKey{{
			APIKey:  "sk-upstream",
			BaseURL: "http://127.0.0.1:1",
		}},
	}
	account := &accountSpec{ID: "api-account", Email: "api@example.com", UpstreamAPIKey: "sk-upstream"}
	m := &manifest{
		Accounts:        []accountSpec{*account},
		accountByID:     map[string]*accountSpec{"api-account": account},
		accountByAuthID: map[string]*accountSpec{},
		accountByAPIKey: map[string]*accountSpec{"sk-upstream": account},
		ModelIDs:        []string{"gpt-5.4"},
	}
	manager := buildCoreAuthManager(cfg, &cockpitSelector{manifest: m}, &authHook{manifest: m})

	runtime, err := newSidecarRuntime(context.Background(), configPath, cfg, m, manager)
	if err != nil {
		t.Fatalf("newSidecarRuntime: %v", err)
	}
	defer runtime.Stop()

	var codexAPIKeyAuth *coreauth.Auth
	for _, auth := range manager.List() {
		if auth == nil || !strings.EqualFold(auth.Provider, "codex") {
			continue
		}
		if auth.Attributes != nil && strings.TrimSpace(auth.Attributes["api_key"]) == "sk-upstream" {
			codexAPIKeyAuth = auth
			break
		}
	}
	if codexAPIKeyAuth == nil {
		t.Fatalf("expected codex API Key auth to be registered, got %#v", manager.List())
	}
	if got := m.accountByAuthID[strings.ToLower(codexAPIKeyAuth.ID)]; got == nil || got.ID != "api-account" {
		t.Fatalf("expected auth to be linked to manifest account, got %#v", got)
	}
}

func TestBuiltinTranslatorNormalizesOpenAIResponsesForCodex(t *testing.T) {
	in := []byte(`{"model":"gpt-5.4-mini","input":"pong","stream":false,"temperature":0.1}`)
	out := sdktranslator.TranslateRequest(
		sdktranslator.FormatOpenAIResponse,
		sdktranslator.FormatCodex,
		"gpt-5.4-mini",
		in,
		true,
	)

	var payload map[string]any
	if err := json.Unmarshal(out, &payload); err != nil {
		t.Fatalf("translated payload should be JSON: %v", err)
	}
	if payload["stream"] != true {
		t.Fatalf("stream should be forced true, got %#v", payload["stream"])
	}
	if _, exists := payload["temperature"]; exists {
		t.Fatalf("unsupported temperature leaked into Codex payload: %s", out)
	}
	input, ok := payload["input"].([]any)
	if !ok || len(input) != 1 {
		t.Fatalf("input should be normalized to a message list, got %#v", payload["input"])
	}
	first, ok := input[0].(map[string]any)
	if !ok || first["type"] != "message" || first["role"] != "user" {
		t.Fatalf("unexpected normalized input item: %#v", input[0])
	}
}

func TestRequestPolicyMiddlewareSetsCPAUsageAPIKey(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := &manifest{
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true},
		},
	}
	policy := &requestPolicy{manifest: m}
	router := gin.New()
	router.Use(policy.middleware())
	router.GET("/v1/responses", func(c *gin.Context) {
		value, exists := c.Get(ginUserAPIKeyKey)
		if !exists {
			t.Fatalf("%s should be set for CPA usage reporter", ginUserAPIKeyKey)
		}
		if value != "client-key" {
			t.Fatalf("unexpected %s: %#v", ginUserAPIKeyKey, value)
		}
		c.Status(http.StatusNoContent)
	})

	req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
	req.Header.Set("Authorization", "Bearer client-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusNoContent {
		t.Fatalf("unexpected status: %d", w.Code)
	}
}

func TestRequestUsageTrackerFinalizesWithLastSuccessfulAttempt(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.record(usagePayload{
		Type:          "usage",
		RequestID:     "req-1",
		AccountID:     "account-failed",
		AccountEmail:  "failed@example.com",
		Model:         "gpt-5.5",
		RequestKind:   "text",
		Success:       false,
		Status:        http.StatusInternalServerError,
		ErrorCategory: "upstream_error",
		ErrorMessage:  "unexpected EOF",
	})
	tracker.record(usagePayload{
		Type:         "usage",
		RequestID:    "req-1",
		AccountID:    "account-ok",
		AccountEmail: "ok@example.com",
		Model:        "gpt-5.5",
		RequestKind:  "text",
		Success:      true,
		Status:       http.StatusOK,
		Usage: usageDetails{
			InputTokens:  10,
			OutputTokens: 5,
			TotalTokens:  15,
		},
	})

	payload, ok := tracker.finalize("req-1", usageFinalizeInput{
		spec:          &apiKeySpec{ID: "key_1", Label: "Default"},
		requestKind:   "text",
		model:         "gpt-5.5",
		status:        http.StatusOK,
		latencyMS:     446_000,
		completedAtMS: 123,
	})

	if !ok {
		t.Fatal("expected finalized usage payload")
	}
	if !payload.Success || payload.AccountID != "account-ok" {
		t.Fatalf("expected successful account payload, got %#v", payload)
	}
	if payload.ErrorCategory != "" || payload.ErrorMessage != "" {
		t.Fatalf("successful final request should not keep attempt error: %#v", payload)
	}
	if payload.LatencyMS != 446_000 || payload.APIKeyID != "key_1" {
		t.Fatalf("final request metadata was not applied: %#v", payload)
	}
}

func TestRequestUsageTrackerKeepsStreamFailureAfterHTTPHeaders(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.record(usagePayload{
		Type:          "usage",
		RequestID:     "req-2",
		AccountID:     "account-failed",
		Model:         "gpt-5.5",
		RequestKind:   "text",
		Success:       false,
		ErrorCategory: "request_failed",
		ErrorMessage:  "stream closed",
	})

	payload, ok := tracker.finalize("req-2", usageFinalizeInput{
		requestKind:   "text",
		model:         "gpt-5.5",
		status:        http.StatusOK,
		latencyMS:     100,
		completedAtMS: 123,
	})

	if !ok {
		t.Fatal("expected finalized usage payload")
	}
	if payload.Success || payload.ErrorCategory != "request_failed" {
		t.Fatalf("stream failure should remain failed even when HTTP status is 200: %#v", payload)
	}
}

func TestRequestPolicyEmitsRequestDiagnostics(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := &manifest{
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true},
		},
	}
	policy := &requestPolicy{manifest: m, emitter: &eventEmitter{}}
	router := gin.New()
	router.Use(policy.middleware())
	router.GET("/v1/responses", func(c *gin.Context) {
		if internallogging.GetRequestID(c.Request.Context()) == "" {
			t.Fatalf("request id should be attached to request context")
		}
		c.Status(http.StatusNoContent)
	})

	out := captureStdout(t, func() {
		req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
		req.Header.Set("Authorization", "Bearer client-key")
		router.ServeHTTP(httptest.NewRecorder(), req)
	})
	lines := strings.Split(strings.TrimSpace(out), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected start and complete diagnostics, got %d lines:\n%s", len(lines), out)
	}
	var start requestDiagnosticPayload
	if err := json.Unmarshal([]byte(lines[0]), &start); err != nil {
		t.Fatalf("start diagnostic should be JSON: %v\n%s", err, lines[0])
	}
	var complete requestDiagnosticPayload
	if err := json.Unmarshal([]byte(lines[1]), &complete); err != nil {
		t.Fatalf("complete diagnostic should be JSON: %v\n%s", err, lines[1])
	}
	if start.Type != "request_started" || complete.Type != "request_completed" {
		t.Fatalf("unexpected diagnostic types: %#v %#v", start.Type, complete.Type)
	}
	if start.RequestID == "" || complete.RequestID != start.RequestID {
		t.Fatalf("request id should be stable across diagnostics: %#v %#v", start, complete)
	}
	if complete.Status != http.StatusNoContent || complete.RequestKind != "text" || complete.APIKeyID != "key_1" {
		t.Fatalf("unexpected completion diagnostic: %#v", complete)
	}
}

func TestUsagePluginResolvesAPIKeyAndRequestKindFromCPARecord(t *testing.T) {
	m := &manifest{
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true},
		},
	}
	tracker := newRequestUsageTracker()
	plugin := &usagePlugin{manifest: m, tracker: tracker}
	ctx := internallogging.WithRequestID(context.Background(), "req-1")
	ctx = internallogging.WithEndpoint(ctx, "POST /v1/responses")

	plugin.HandleUsage(ctx, coreusage.Record{
		Provider:    "codex",
		Model:       "gpt-5.4-mini",
		APIKey:      "client-key",
		RequestedAt: time.UnixMilli(123),
		Latency:     50 * time.Millisecond,
	})

	payload, ok := tracker.finalize("req-1", usageFinalizeInput{
		status:        http.StatusOK,
		latencyMS:     50,
		completedAtMS: 123,
	})
	if !ok {
		t.Fatal("expected usage payload")
	}
	if payload.APIKeyID != "key_1" || payload.APIKeyLabel != "Test key" {
		t.Fatalf("API key metadata was not resolved: %#v", payload)
	}
	if payload.RequestID != "req-1" {
		t.Fatalf("request id should be forwarded, got %q", payload.RequestID)
	}
	if payload.RequestKind != "text" {
		t.Fatalf("request kind should be inferred from endpoint, got %q", payload.RequestKind)
	}
}

func TestErrorCategoryClassifiesClientCanceled(t *testing.T) {
	if got := errorCategory(0, "context canceled", false); got != "client_canceled" {
		t.Fatalf("expected client_canceled, got %q", got)
	}
	if got := errorCategory(http.StatusGatewayTimeout, `Post "https://chatgpt.com/backend-api/codex/responses": context canceled`, false); got != "gateway_context_canceled" {
		t.Fatalf("expected gateway_context_canceled for upstream context cancellation, got %q", got)
	}
	if got := errorCategory(http.StatusBadGateway, "write tcp: broken pipe", false); got != "client_canceled" {
		t.Fatalf("expected client_canceled for broken pipe, got %q", got)
	}
	if got := errorCategory(http.StatusGatewayTimeout, "upstream timed out in stream_open attempt=1/1 after 60s", false); got != "upstream_first_byte_timeout" {
		t.Fatalf("expected upstream_first_byte_timeout, got %q", got)
	}
}

func TestAuthHookEmitsRequestScopedResultDiagnostics(t *testing.T) {
	apiKey := &apiKeySpec{ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true}
	account := &accountSpec{ID: "account_1", Email: "user@example.com", AuthID: "auth.json"}
	m := &manifest{
		accountByAuthID: map[string]*accountSpec{"auth.json": account},
		accountByID:     map[string]*accountSpec{"auth": account},
	}
	hook := &authHook{manifest: m, emitter: &eventEmitter{}}
	ctx := internallogging.WithRequestID(context.Background(), "req-2")
	ctx = context.WithValue(ctx, clientAPIKeyContextKey, apiKey)
	ctx = context.WithValue(ctx, requestKindContextKey, "text")
	ctx = context.WithValue(ctx, requestModelContextKey, "gpt-5.5")

	out := captureStdout(t, func() {
		hook.OnResult(ctx, coreauth.Result{
			AuthID:   "auth.json",
			Provider: "codex",
			Model:    "upstream-model",
			Success:  false,
			Error: &coreauth.Error{
				Code:       "upstream_timeout",
				Message:    "upstream timed out",
				Retryable:  true,
				HTTPStatus: http.StatusGatewayTimeout,
			},
		})
	})

	var payload requestDiagnosticPayload
	if err := json.Unmarshal([]byte(out), &payload); err != nil {
		t.Fatalf("auth result diagnostic should be JSON: %v\n%s", err, out)
	}
	if payload.Type != "auth_result" || payload.RequestID != "req-2" {
		t.Fatalf("unexpected auth result diagnostic identity: %#v", payload)
	}
	if payload.Model != "gpt-5.5" || payload.AccountID != "account_1" || payload.APIKeyID != "key_1" {
		t.Fatalf("unexpected auth result metadata: %#v", payload)
	}
	if payload.Success == nil || *payload.Success || payload.Retryable == nil || !*payload.Retryable {
		t.Fatalf("failure details should be preserved: %#v", payload)
	}
	if payload.HTTPStatus != http.StatusGatewayTimeout || payload.ErrorCode != "upstream_timeout" {
		t.Fatalf("unexpected failure details: %#v", payload)
	}
}

func TestRelayServerExecutesNonStreamingRequestThroughRuntime(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{
		response: cliproxyexecutor.Response{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Payload: []byte(`{"ok":true}`),
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if strings.TrimSpace(w.Body.String()) != `{"ok":true}` {
		t.Fatalf("unexpected body: %s", w.Body.String())
	}
	if runtime.executeCalls != 1 || runtime.streamCalls != 0 {
		t.Fatalf("unexpected runtime calls: execute=%d stream=%d", runtime.executeCalls, runtime.streamCalls)
	}
	if runtime.lastReq.Model != "gpt-5.5" || runtime.lastOpts.SourceFormat != sdktranslator.FormatOpenAIResponse {
		t.Fatalf("unexpected executor request: %#v %#v", runtime.lastReq, runtime.lastOpts)
	}
	if runtime.lastOpts.Headers.Get("Authorization") != "Bearer client-key" {
		t.Fatalf("request headers should be forwarded to CPA executor")
	}
	if w.Header().Get("Access-Control-Allow-Origin") != "*" {
		t.Fatalf("CORS header should match CPA server behavior")
	}
}

func TestRelayServerAcceptsCodexAutoReviewModel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{
		response: cliproxyexecutor.Response{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Payload: []byte(`{"ok":true}`),
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"codex-auto-review","input":"allow?","stream":false}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.executeCalls != 1 || runtime.lastReq.Model != codexAutoReviewModel {
		t.Fatalf("auto review request should be forwarded unchanged: calls=%d req=%#v", runtime.executeCalls, runtime.lastReq)
	}
}

func TestRelayServerModelsExposeCodexAutoReview(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := testRelayRouter(&fakeRuntime{})

	req := httptest.NewRequest(http.MethodGet, "/v1/models", nil)
	req.Header.Set("Authorization", "Bearer client-key")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), codexAutoReviewModel) {
		t.Fatalf("models response should expose auto review model: %s", w.Body.String())
	}
}

func TestRelayServerFramesStreamingChatCompletionThroughRuntime(t *testing.T) {
	gin.SetMode(gin.TestMode)
	stream := make(chan cliproxyexecutor.StreamChunk, 2)
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`{"choices":[]}`)}
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`[DONE]`)}
	close(stream)
	runtime := &fakeRuntime{
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{
				"Content-Type":       []string{"application/json"},
				"Connection":         []string{"X-Remove-Me"},
				"X-Remove-Me":        []string{"secret"},
				"X-Litellm-Trace":    []string{"gateway"},
				"Content-Encoding":   []string{"gzip"},
				"X-Upstream":         []string{"ok"},
				"Access-Control-Foo": []string{"bar"},
			},
			Chunks: stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(`{"model":"gpt-5.5","messages":[],"stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.executeCalls != 0 || runtime.streamCalls != 1 {
		t.Fatalf("unexpected runtime calls: execute=%d stream=%d", runtime.executeCalls, runtime.streamCalls)
	}
	if runtime.lastOpts.SourceFormat != sdktranslator.FormatOpenAI || !runtime.lastOpts.Stream {
		t.Fatalf("unexpected stream options: %#v", runtime.lastOpts)
	}
	if got := w.Header().Get("Content-Type"); !strings.HasPrefix(got, "text/event-stream") {
		t.Fatalf("unexpected content type: %q", got)
	}
	if values := w.Header().Values("Content-Type"); len(values) != 1 {
		t.Fatalf("Content-Type should not be duplicated: %#v", values)
	}
	if w.Header().Get("X-Upstream") != "ok" {
		t.Fatalf("upstream headers should be preserved")
	}
	if w.Header().Get("X-Remove-Me") != "" ||
		w.Header().Get("X-Litellm-Trace") != "" ||
		w.Header().Get("Content-Encoding") != "" {
		t.Fatalf("filtered upstream headers leaked: %#v", w.Header())
	}
	if got := w.Body.String(); got != "data: {\"choices\":[]}\n\ndata: [DONE]\n\n" {
		t.Fatalf("unexpected framed stream:\n%s", got)
	}
}

func TestRelayServerTimesOutWhenStreamDoesNotOpen(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldTimeout := streamOpenTimeout
	oldAttempts := streamOpenMaxAttempts
	streamOpenTimeout = 20 * time.Millisecond
	streamOpenMaxAttempts = 2
	defer func() {
		streamOpenTimeout = oldTimeout
		streamOpenMaxAttempts = oldAttempts
	}()
	router := testRelayRouter(&fakeRuntime{streamWaitForContext: true})

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusGatewayTimeout {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "stream_open") {
		t.Fatalf("timeout response should name stream_open phase: %s", w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "upstream_first_byte_timeout") {
		t.Fatalf("timeout response should expose first-byte timeout code: %s", w.Body.String())
	}
}

func TestRelayServerUsesLongOpenTimeoutForImageGenerationTool(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldOpenTimeout := streamOpenTimeout
	oldImageOpenTimeout := imageStreamOpenTimeout
	oldAttempts := streamOpenMaxAttempts
	streamOpenTimeout = 20 * time.Millisecond
	imageStreamOpenTimeout = 120 * time.Millisecond
	streamOpenMaxAttempts = 1
	defer func() {
		streamOpenTimeout = oldOpenTimeout
		imageStreamOpenTimeout = oldImageOpenTimeout
		streamOpenMaxAttempts = oldAttempts
	}()
	stream := make(chan cliproxyexecutor.StreamChunk, 1)
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`event: response.completed
data: {"type":"response.completed"}

`)}
	close(stream)
	runtime := &fakeRuntime{
		streamOpenDelay: 60 * time.Millisecond,
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"text/event-stream"}},
			Chunks:  stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"draw","stream":true,"tools":[{"type":"image_generation","model":"gpt-image-2"}]}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("image stream should use longer open timeout, got status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.streamCalls != 1 {
		t.Fatalf("expected one stream runtime call, got %d", runtime.streamCalls)
	}
	if !strings.Contains(w.Body.String(), "response.completed") {
		t.Fatalf("image stream response was not forwarded: %s", w.Body.String())
	}
}

func TestRelayServerHandlesImagesGenerationsEndpoint(t *testing.T) {
	gin.SetMode(gin.TestMode)
	stream := make(chan cliproxyexecutor.StreamChunk, 1)
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`event: response.completed
data: {"type":"response.completed","response":{"created_at":1710000000,"output":[{"type":"image_generation_call","result":"ZmFrZS1wbmc=","output_format":"png","size":"1024x1024"}]}}

`)}
	close(stream)
	runtime := &fakeRuntime{
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"text/event-stream"}},
			Chunks:  stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/images/generations", strings.NewReader(`{"model":"gpt-image-2","prompt":"draw","response_format":"b64_json"}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.streamCalls != 1 || runtime.executeCalls != 0 {
		t.Fatalf("unexpected runtime calls: execute=%d stream=%d", runtime.executeCalls, runtime.streamCalls)
	}
	if runtime.lastReq.Model != defaultImagesMainModel {
		t.Fatalf("image endpoint should execute via main model, got %q", runtime.lastReq.Model)
	}
	var body map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatalf("response should be json: %v body=%s", err, w.Body.String())
	}
	data, _ := body["data"].([]any)
	if len(data) != 1 {
		t.Fatalf("expected one image result: %#v", body)
	}
	first, _ := data[0].(map[string]any)
	if first["b64_json"] != "ZmFrZS1wbmc=" {
		t.Fatalf("unexpected image payload: %#v", body)
	}
}

func TestRelayServerRetriesWhenStreamDoesNotOpen(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldTimeout := streamOpenTimeout
	oldAttempts := streamOpenMaxAttempts
	streamOpenTimeout = 20 * time.Millisecond
	streamOpenMaxAttempts = 2
	defer func() {
		streamOpenTimeout = oldTimeout
		streamOpenMaxAttempts = oldAttempts
	}()
	stream := make(chan cliproxyexecutor.StreamChunk, 1)
	stream <- cliproxyexecutor.StreamChunk{Payload: []byte(`[DONE]`)}
	close(stream)
	runtime := &fakeRuntime{
		streamWaitAttempts: 1,
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Chunks:  stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.streamCalls != 2 {
		t.Fatalf("expected retry to call stream runtime twice, got %d", runtime.streamCalls)
	}
	if !strings.Contains(w.Body.String(), "[DONE]") {
		t.Fatalf("retry should stream successful second attempt: %s", w.Body.String())
	}
}

func TestRelayServerKeepsStreamContextOpenAfterOpen(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldOpenTimeout := streamOpenTimeout
	oldIdleTimeout := streamIdleTimeout
	streamOpenTimeout = 100 * time.Millisecond
	streamIdleTimeout = time.Second
	defer func() {
		streamOpenTimeout = oldOpenTimeout
		streamIdleTimeout = oldIdleTimeout
	}()
	runtime := &fakeRuntime{
		streamResultFromContext: true,
		streamResultDelay:       20 * time.Millisecond,
		streamResultPayload:     []byte(`[DONE]`),
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("unexpected status: %d body=%s", w.Code, w.Body.String())
	}
	if runtime.streamCalls != 1 {
		t.Fatalf("expected one stream runtime call, got %d", runtime.streamCalls)
	}
	if !strings.Contains(w.Body.String(), "[DONE]") {
		t.Fatalf("stream context should stay alive after opening: %s", w.Body.String())
	}
}

func TestRelayServerTimesOutIdleOpenedStream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	oldTimeout := streamIdleTimeout
	streamIdleTimeout = 20 * time.Millisecond
	defer func() {
		streamIdleTimeout = oldTimeout
	}()
	stream := make(chan cliproxyexecutor.StreamChunk)
	runtime := &fakeRuntime{
		streamResult: &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Chunks:  stream,
		},
	}
	router := testRelayRouter(runtime)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.5","input":"hello","stream":true}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("stream should be opened before idle timeout, got status: %d body=%s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "stream_idle") {
		t.Fatalf("idle timeout should be sent as terminal SSE error: %s", w.Body.String())
	}
}

func TestRelayServerHandlesCORSPreflight(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := testRelayRouter(&fakeRuntime{})

	req := httptest.NewRequest(http.MethodOptions, "/v1/responses", nil)
	req.Header.Set("Access-Control-Request-Headers", "authorization,content-type")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusNoContent {
		t.Fatalf("unexpected status: %d", w.Code)
	}
	if w.Header().Get("Access-Control-Allow-Origin") != "*" ||
		w.Header().Get("Access-Control-Allow-Headers") != "*" {
		t.Fatalf("unexpected CORS headers: %#v", w.Header())
	}
}

func testRelayRouter(runtime executorRuntime) *gin.Engine {
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true}},
		ModelIDs: []string{"gpt-5.5", "gpt-image-2"},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "key_1", Label: "Test key", Key: "client-key", Enabled: true},
		},
	}
	policy := &requestPolicy{manifest: m}
	return (&relayServer{
		runtime:  runtime,
		cfg:      &config.Config{},
		manifest: m,
		policy:   policy,
	}).router()
}

type fakeRuntime struct {
	response                cliproxyexecutor.Response
	streamResult            *cliproxyexecutor.StreamResult
	err                     error
	streamWaitForContext    bool
	streamWaitAttempts      int
	streamResultFromContext bool
	streamOpenDelay         time.Duration
	streamResultDelay       time.Duration
	streamResultPayload     []byte

	executeCalls int
	streamCalls  int
	lastReq      cliproxyexecutor.Request
	lastOpts     cliproxyexecutor.Options
}

func (r *fakeRuntime) Execute(_ context.Context, _ []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	r.executeCalls++
	r.lastReq = req
	r.lastOpts = opts
	return r.response, r.err
}

func (r *fakeRuntime) ExecuteStream(ctx context.Context, _ []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	r.streamCalls++
	r.lastReq = req
	r.lastOpts = opts
	if r.streamWaitForContext || r.streamCalls <= r.streamWaitAttempts {
		<-ctx.Done()
		return nil, ctx.Err()
	}
	if r.streamOpenDelay > 0 {
		timer := time.NewTimer(r.streamOpenDelay)
		defer timer.Stop()
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-timer.C:
		}
	}
	if r.streamResultFromContext {
		stream := make(chan cliproxyexecutor.StreamChunk, 1)
		delay := r.streamResultDelay
		if delay <= 0 {
			delay = 10 * time.Millisecond
		}
		payload := r.streamResultPayload
		if len(payload) == 0 {
			payload = []byte(`[DONE]`)
		}
		go func() {
			defer close(stream)
			timer := time.NewTimer(delay)
			defer timer.Stop()
			select {
			case <-ctx.Done():
				return
			case <-timer.C:
				stream <- cliproxyexecutor.StreamChunk{Payload: payload}
			}
		}()
		return &cliproxyexecutor.StreamResult{
			Headers: http.Header{"Content-Type": []string{"application/json"}},
			Chunks:  stream,
		}, nil
	}
	return r.streamResult, r.err
}

func captureStdout(t *testing.T, fn func()) string {
	t.Helper()
	old := os.Stdout
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("create stdout pipe: %v", err)
	}
	os.Stdout = writer
	defer func() {
		os.Stdout = old
		_ = reader.Close()
	}()

	fn()
	if err := writer.Close(); err != nil {
		t.Fatalf("close stdout pipe: %v", err)
	}
	data, err := io.ReadAll(reader)
	if err != nil {
		t.Fatalf("read stdout pipe: %v", err)
	}
	return string(data)
}
