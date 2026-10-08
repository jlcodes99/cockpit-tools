package main

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

// Exercise HTTP policy and the real account selector without contacting an upstream.
type nativeRouteTestRuntime struct {
	fakeRuntime
	selector       *cockpitSelector
	auths          []*coreauth.Auth
	selected       *coreauth.Auth
	executionKey   *apiKeySpec
	providers      []string
	selectionCalls int
}

func (r *nativeRouteTestRuntime) selectAuth(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) error {
	r.selectionCalls++
	r.providers = append([]string(nil), providers...)
	r.executionKey, _ = ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	var candidates []*coreauth.Auth
	for _, auth := range r.auths {
		for _, provider := range providers {
			if auth.Provider == provider {
				candidates = append(candidates, auth)
			}
		}
	}
	selected, err := r.selector.Pick(ctx, providers[0], req.Model, opts, candidates)
	if err == nil {
		r.selected = selected
	}
	return err
}

func (r *nativeRouteTestRuntime) Execute(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	if err := r.selectAuth(ctx, providers, req, opts); err != nil {
		return cliproxyexecutor.Response{}, err
	}
	return r.fakeRuntime.Execute(ctx, providers, req, opts)
}

func (r *nativeRouteTestRuntime) ExecuteStream(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	if err := r.selectAuth(ctx, providers, req, opts); err != nil {
		return nil, err
	}
	return r.fakeRuntime.ExecuteStream(ctx, providers, req, opts)
}

func newNativeRouteTestRouter() (*gin.Engine, *nativeRouteTestRuntime, *apiKeySpec) {
	gin.SetMode(gin.TestMode)
	spec := &apiKeySpec{
		ID: "mixed-key", Key: "mixed-secret", Enabled: true, BoundOAuth: true,
		AccountIDs: []string{"oauth-default"}, TokenLimit: 100, TokenUsed: 7,
		ModelRouting: &modelRoutingSpec{
			DefaultRoute: "oauth", FailurePolicy: "strict",
			Routes: []modelRouteSpec{{
				ID: "grok-route", Namespace: "grok", NativeProvider: "xai", ProviderAccountID: "grok-target",
				ProviderGateway: &providerGatewaySpec{UpstreamModel: "grok-4.6", UpstreamModels: []string{"grok-4.6"}},
			}},
		},
	}
	m := &manifest{
		ModelIDs: []string{"gpt-5.5"}, APIKeys: []apiKeySpec{*spec},
		apiKeyByValue: map[string]*apiKeySpec{spec.Key: spec},
		accountByID:   make(map[string]*accountSpec), accountByAuthID: make(map[string]*accountSpec),
	}
	// Put the unrelated Grok account first: the request must still pick the bound one.
	for _, account := range []accountSpec{
		{ID: "grok-other", AuthID: "other.json", Provider: "xai"},
		{ID: "grok-target", AuthID: "target.json", Provider: "xai"},
		{ID: "oauth-default", AuthID: "oauth.json", Provider: "codex"},
	} {
		m.accountByID[account.ID] = &account
		m.accountByAuthID[account.AuthID] = &account
	}
	chunks := make(chan cliproxyexecutor.StreamChunk, 1)
	chunks <- cliproxyexecutor.StreamChunk{Payload: []byte("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-native\",\"status\":\"completed\",\"output\":[]}}\n\n")}
	close(chunks)
	runtime := &nativeRouteTestRuntime{
		fakeRuntime: fakeRuntime{
			response:     cliproxyexecutor.Response{Payload: []byte(`{"id":"resp-native","status":"completed","output":[]}`)},
			streamResult: &cliproxyexecutor.StreamResult{Chunks: chunks},
		},
		selector: &cockpitSelector{manifest: m},
		auths: []*coreauth.Auth{
			{ID: "other.json", Provider: "xai", Status: coreauth.StatusActive},
			{ID: "target.json", Provider: "xai", Status: coreauth.StatusActive},
			{ID: "oauth.json", Provider: "codex", Status: coreauth.StatusActive},
		},
	}
	server := &relayServer{runtime: runtime, cfg: &config.Config{}, manifest: m, policy: &requestPolicy{manifest: m, tokenLimiter: newAPIKeyTokenLimiter(nil)}}
	return server.router(), runtime, spec
}

func nativeRouteTestRequest(router http.Handler, key, model string, stream bool) *httptest.ResponseRecorder {
	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(fmt.Sprintf(`{"model":%q,"input":"hello","stream":%t}`, model, stream)))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+key)
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	return w
}

func TestNativeProviderRouteUsesOnlyBoundAccount(t *testing.T) {
	for _, stream := range []bool{false, true} {
		t.Run(fmt.Sprintf("stream=%t", stream), func(t *testing.T) {
			router, runtime, spec := newNativeRouteTestRouter()
			before := *spec
			w := nativeRouteTestRequest(router, spec.Key, "grok/grok-4.6", stream)
			if w.Code != http.StatusOK || runtime.selected == nil || runtime.selected.ID != "target.json" {
				t.Fatalf("bound Grok account not selected: status=%d selected=%v body=%s", w.Code, runtime.selected, w.Body.String())
			}
			if !reflect.DeepEqual(runtime.providers, []string{"xai"}) || runtime.lastReq.Model != "grok-4.6" {
				t.Fatalf("unexpected native execution: providers=%v model=%s", runtime.providers, runtime.lastReq.Model)
			}
			if runtime.lastOpts.Stream != stream || runtime.executeCalls+runtime.streamCalls != 1 {
				t.Fatalf("wrong execution mode: opts=%v execute=%d stream=%d", runtime.lastOpts.Stream, runtime.executeCalls, runtime.streamCalls)
			}
			if runtime.executionKey == spec || !reflect.DeepEqual(runtime.executionKey.AccountIDs, []string{"grok-target"}) {
				t.Fatalf("route must use its own singleton account scope: %#v", runtime.executionKey)
			}
			executionIdentity := *runtime.executionKey
			executionIdentity.AccountIDs = before.AccountIDs
			if !reflect.DeepEqual(executionIdentity, before) || !reflect.DeepEqual(*spec, before) {
				t.Fatal("native scope changed API-key identity, budget, policy or shared configuration")
			}
			w = nativeRouteTestRequest(router, spec.Key, "gpt-5.5", false)
			if w.Code != http.StatusOK || runtime.selected.ID != "oauth.json" {
				t.Fatalf("default OAuth request was polluted by native route: status=%d selected=%v body=%s", w.Code, runtime.selected, w.Body.String())
			}
		})
	}
}

func TestNativeProviderRouteDoesNotFallbackToOtherAccount(t *testing.T) {
	for _, stream := range []bool{false, true} {
		for _, unavailable := range []string{"missing", "disabled", "missing_binding"} {
			t.Run(fmt.Sprintf("stream=%t/%s", stream, unavailable), func(t *testing.T) {
				router, runtime, spec := newNativeRouteTestRouter()
				switch unavailable {
				case "missing":
					runtime.auths = append(runtime.auths[:1], runtime.auths[2:]...)
				case "disabled":
					runtime.auths[1].Disabled = true
				case "missing_binding":
					spec.ModelRouting.Routes[0].ProviderAccountID = ""
				}
				w := nativeRouteTestRequest(router, spec.Key, "grok/grok-4.6", stream)
				if w.Code == http.StatusOK || runtime.selected != nil || runtime.executeCalls+runtime.streamCalls != 0 {
					t.Fatalf("unavailable route must not fall back: status=%d selected=%v body=%s", w.Code, runtime.selected, w.Body.String())
				}
			})
		}
	}
}

func TestNativeProviderRoutePreservesAPIKeyGuards(t *testing.T) {
	for _, guard := range []string{"invalid_key", "token_limit", "excluded_model", "unrouted_key"} {
		t.Run(guard, func(t *testing.T) {
			router, runtime, spec := newNativeRouteTestRouter()
			key := spec.Key
			wantStatus := http.StatusNotFound
			switch guard {
			case "invalid_key":
				key = "wrong-key"
				wantStatus = http.StatusUnauthorized
			case "token_limit":
				spec.TokenUsed = spec.TokenLimit
				wantStatus = http.StatusTooManyRequests
			case "excluded_model":
				spec.ExcludedModels = []string{"grok/grok-4.6"}
			case "unrouted_key":
				spec.ModelRouting = nil
			}
			w := nativeRouteTestRequest(router, key, "grok/grok-4.6", false)
			if w.Code != wantStatus || runtime.selectionCalls != 0 {
				t.Fatalf("guard %s bypassed: status=%d want=%d selectionCalls=%d body=%s", guard, w.Code, wantStatus, runtime.selectionCalls, w.Body.String())
			}
		})
	}
}
