package main

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

const automaticRoutingManifestPayload = `{
	"modelIds": ["gpt-5.5"],
	"accounts": [
		{"id": "native-account", "email": "native@example.com", "authId": "native-account.json"},
		{"id": "chat-account", "email": "chat@example.com", "upstreamApiKey": "sk-chat"}
	],
	"apiKeys": [{
		"id": "client",
		"label": "Client",
		"key": "client-key",
		"enabled": true,
		"accountIds": ["native-account", "chat-account"],
		"allowedModels": [],
		"excludedModels": [],
		"modelRouting": {
			"automatic": true,
			"nativeModels": ["gpt-5.5", " shared "],
			"defaultRoute": "oauth",
			"failurePolicy": "strict",
			"routes": [{
				"id": "auto-chat",
				"namespace": "api-chat",
				"providerAccountId": "chat-account",
				"providerGateway": {
					"baseUrl": "http://127.0.0.1:1/v1",
					"apiKey": "sk-chat",
					"upstreamModel": "vendor/real",
					"upstreamModels": ["vendor/real", "deepseek-chat"],
					"wireApi": "chat_completions"
				},
				"models": [
					{"clientModel": " shared ", "upstreamModel": " vendor/real "},
					{"clientModel": "SHARED", "upstreamModel": "vendor/other"},
					{"clientModel": "deepseek-chat", "upstreamModel": "deepseek-chat"}
				]
			}]
		}
	}]
}`

func loadAutomaticRoutingManifest(t *testing.T, payload string) *manifest {
	t.Helper()
	path := filepath.Join(t.TempDir(), "manifest.json")
	if err := os.WriteFile(path, []byte(payload), 0o644); err != nil {
		t.Fatalf("write manifest: %v", err)
	}
	m, err := loadManifest(path)
	if err != nil {
		t.Fatalf("load manifest: %v", err)
	}
	return m
}

func TestLoadManifestNormalizesAutomaticModelRouting(t *testing.T) {
	m := loadAutomaticRoutingManifest(t, automaticRoutingManifestPayload)
	spec := m.apiKeyByValue["client-key"]
	if spec == nil || spec.ModelRouting == nil {
		t.Fatalf("automatic routing should be retained: %#v", spec)
	}
	routing := spec.ModelRouting
	if !routing.Automatic {
		t.Fatal("automatic flag must survive manifest loading")
	}
	if got := strings.Join(routing.NativeModels, ","); got != "gpt-5.5,shared" {
		t.Fatalf("native models = %q, want trimmed list", got)
	}
	if len(routing.Routes) != 1 {
		t.Fatalf("routes = %d, want 1", len(routing.Routes))
	}
	models := routing.Routes[0].Models
	if len(models) != 2 {
		t.Fatalf("route models = %#v, want duplicates removed", models)
	}
	if models[0].ClientModel != "shared" || models[0].UpstreamModel != "vendor/real" {
		t.Fatalf("first route model = %#v, want trimmed shared/vendor/real", models[0])
	}
	if models[1].ClientModel != "deepseek-chat" {
		t.Fatalf("second route model = %#v, want deepseek-chat", models[1])
	}
}

func TestLoadManifestKeepsAutomaticRoutingWithoutRoutes(t *testing.T) {
	m := loadAutomaticRoutingManifest(t, `{
		"modelIds": ["gpt-5.5"],
		"apiKeys": [{
			"id": "client", "label": "Client", "key": "client-key", "enabled": true,
			"accountIds": ["native-account"], "allowedModels": [], "excludedModels": [],
			"modelRouting": {
				"automatic": true, "nativeModels": ["gpt-5.5"],
				"defaultRoute": "oauth", "failurePolicy": "strict", "routes": []
			}
		}]
	}`)
	if m.apiKeyByValue["client-key"].ModelRouting == nil {
		t.Fatal("automatic routing without provider routes must not be dropped")
	}
}

func TestAutomaticRoutingListsUnionOfNativeAndProviderModels(t *testing.T) {
	m := loadAutomaticRoutingManifest(t, automaticRoutingManifestPayload)
	spec := m.apiKeyByValue["client-key"]

	want := []string{"gpt-5.5", "shared", "deepseek-chat"}
	if got := visibleModelsForAPIKey(m, spec); strings.Join(got, ",") != strings.Join(want, ",") {
		t.Fatalf("visible models = %#v, want %#v", got, want)
	}

	if !automaticNativeModel(spec, " Gpt-5.5 ") {
		t.Fatal("native model lookup must ignore case and padding")
	}
	if !automaticNativeModel(spec, "SHARED") {
		t.Fatal("models listed as native must stay native regardless of case")
	}
	if automaticNativeModel(spec, "deepseek-chat") {
		t.Fatal("provider-only model must not be treated as native")
	}

	if route, upstream, status := resolveModelRoutingRoute(spec, "gpt-5.5"); route != nil || status != "none" {
		t.Fatalf("native model resolution = (%v, %q, %q), want native pass-through", route, upstream, status)
	}
	// 同名模型同时存在原生账号与 Provider 路由时，仍优先走原生执行器。
	if route, upstream, status := resolveModelRoutingRoute(spec, "SHARED"); route != nil || status != "none" {
		t.Fatalf("overlapping native model resolution = (%v, %q, %q), want native pass-through", route, upstream, status)
	}
	route, upstream, status := resolveModelRoutingRoute(spec, "deepseek-chat")
	if status != "matched" || upstream != "deepseek-chat" || route == nil || route.ProviderAccountID != "chat-account" {
		t.Fatalf("deepseek resolution = (%v, %q, %q), want matched provider route", route, upstream, status)
	}
	if _, _, status := resolveModelRoutingRoute(spec, "unknown-model"); status != "missing" {
		t.Fatalf("unknown model status = %q, want missing", status)
	}
}

func TestAutomaticRoutingCandidatesRespectModelMapping(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := loadAutomaticRoutingManifest(t, automaticRoutingManifestPayload)
	spec := m.apiKeyByValue["client-key"]
	server := &relayServer{manifest: m, policy: &requestPolicy{manifest: m}}

	shared := server.automaticCandidates(spec, "shared")
	if len(shared) != 1 {
		t.Fatalf("shared candidates = %#v, want single chat route", shared)
	}
	if shared[0].upstream != "vendor/real" || shared[0].route.ProviderAccountID != "chat-account" {
		t.Fatalf("shared candidate = %#v, want vendor/real on chat-account", shared[0])
	}
	if candidates := server.automaticCandidates(spec, "gpt-5.5"); len(candidates) != 0 {
		t.Fatalf("native model must not use provider candidates: %#v", candidates)
	}
}

func TestRewriteBodyModelValidatesAutomaticModelScope(t *testing.T) {
	m := loadAutomaticRoutingManifest(t, automaticRoutingManifestPayload)
	spec := m.apiKeyByValue["client-key"]

	body := []byte(`{"model":"deepseek-chat","messages":[{"role":"user","content":"hi"}]}`)
	rewritten, model, err := rewriteBodyModel(m, spec, "text", body)
	if err != nil {
		t.Fatalf("provider model should be accepted: %v", err)
	}
	if rewritten != nil || model != "deepseek-chat" {
		t.Fatalf("automatic routing must keep the client model, got (%s, %q)", rewritten, model)
	}
	if _, _, err := rewriteBodyModel(m, spec, "text", []byte(`{"model":"missing-model"}`)); err == nil {
		t.Fatal("model outside the automatic pool must be rejected")
	}
}

type firstAuthSelector struct {
	calls int32
}

func (s *firstAuthSelector) Pick(_ context.Context, _ string, _ string, _ cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	atomic.AddInt32(&s.calls, 1)
	if len(auths) == 0 {
		return nil, errors.New("no candidate auths")
	}
	return auths[0], nil
}

func automaticRoutingTestServer(t *testing.T, runtime *fakeRuntime, upstreamURL string) *gin.Engine {
	t.Helper()
	payload := strings.Replace(automaticRoutingManifestPayload, "http://127.0.0.1:1/v1", upstreamURL, 1)
	m := loadAutomaticRoutingManifest(t, payload)
	policy := &requestPolicy{manifest: m, tracker: newRequestUsageTracker()}
	server := &relayServer{
		runtime:           runtime,
		cfg:               &config.Config{},
		manifest:          m,
		policy:            policy,
		automaticSelector: &firstAuthSelector{},
	}
	return server.router()
}

func postAutomaticRoutingRequest(t *testing.T, router *gin.Engine, model string) *httptest.ResponseRecorder {
	t.Helper()
	body := `{"model":"` + model + `","messages":[{"role":"user","content":"hi"}]}`
	request := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	request.Header.Set("Authorization", "Bearer client-key")
	request.Header.Set("Content-Type", "application/json")
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)
	return recorder
}

func TestAutomaticRoutingPrefersNativeExecutorForOverlappingModel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamCalls int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		atomic.AddInt32(&upstreamCalls, 1)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chat-upstream"}`))
	}))
	defer upstream.Close()

	runtime := &fakeRuntime{response: cliproxyexecutor.Response{Payload: []byte(`{"id":"native-upstream"}`)}}
	router := automaticRoutingTestServer(t, runtime, upstream.URL)
	recorder := postAutomaticRoutingRequest(t, router, "shared")

	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", recorder.Code, recorder.Body.String())
	}
	if !strings.Contains(recorder.Body.String(), "native-upstream") {
		t.Fatalf("overlapping model should be served by the native pool first: %s", recorder.Body.String())
	}
	if calls := atomic.LoadInt32(&upstreamCalls); calls != 0 {
		t.Fatalf("provider gateway calls = %d, want 0 when native pool succeeds", calls)
	}
	if runtime.executeCalls != 1 || runtime.streamCalls != 0 {
		t.Fatalf("executor calls = (%d non-stream, %d stream), want a single native attempt", runtime.executeCalls, runtime.streamCalls)
	}
}

func TestAutomaticRoutingFailsOverToProviderRouteBeforeResponseStarts(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamCalls int32
	var upstreamModel string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&upstreamCalls, 1)
		if r.URL.Path != "/v1/chat/completions" {
			t.Errorf("upstream path = %s, want /v1/chat/completions", r.URL.Path)
		}
		body, _ := io.ReadAll(r.Body)
		upstreamModel = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chat-upstream"}`))
	}))
	defer upstream.Close()

	runtime := &fakeRuntime{err: errors.New("native executor unavailable")}
	router := automaticRoutingTestServer(t, runtime, upstream.URL)
	recorder := postAutomaticRoutingRequest(t, router, "shared")

	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", recorder.Code, recorder.Body.String())
	}
	if !strings.Contains(recorder.Body.String(), "chat-upstream") {
		t.Fatalf("failed native attempt must fall back to the provider route: %s", recorder.Body.String())
	}
	if calls := atomic.LoadInt32(&upstreamCalls); calls != 1 {
		t.Fatalf("provider gateway calls = %d, want 1", calls)
	}
	if !strings.Contains(upstreamModel, "vendor/real") {
		t.Fatalf("provider request model = %s, want rewritten upstream model", upstreamModel)
	}
}

func TestAutomaticRoutingRejectsModelsOutsideThePool(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{}
	router := automaticRoutingTestServer(t, runtime, "http://127.0.0.1:1/v1")
	recorder := postAutomaticRoutingRequest(t, router, "not-a-pool-model")

	if recorder.Code != http.StatusNotFound {
		t.Fatalf("status = %d, body = %s", recorder.Code, recorder.Body.String())
	}
	if runtime.executeCalls != 0 || runtime.streamCalls != 0 {
		t.Fatalf("executor must not run for unknown models: %#v", runtime)
	}
}

const trimmedAutomaticRoutingManifestPayload = `{
	"modelIds": ["gpt-5.5"],
	"accounts": [
		{"id": "native-account", "email": "native@example.com", "authId": "native-account.json"},
		{"id": "chat-account", "email": "chat@example.com", "upstreamApiKey": "sk-chat"}
	],
	"apiKeys": [{
		"id": "client",
		"label": "Client",
		"key": "client-key",
		"enabled": true,
		"accountIds": ["native-account", "chat-account"],
		"allowedModels": [],
		"excludedModels": [],
		"modelRouting": {
			"automatic": true,
			"nativeModels": [
				"gpt-6-astra", "gpt-6-sol", "gpt-6-luna", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5",
				"gpt-image-2.5", "codex-auto-review", "gpt-reserve"
			],
			"routableModels": ["gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex"],
			"defaultRoute": "oauth",
			"failurePolicy": "strict",
			"routes": [{
				"id": "auto-chat",
				"namespace": "api-chat",
				"providerAccountId": "chat-account",
				"providerGateway": {
					"baseUrl": "http://127.0.0.1:1/v1",
					"apiKey": "sk-chat",
					"upstreamModel": "deepseek-chat",
					"upstreamModels": ["deepseek-chat"],
					"wireApi": "chat_completions"
				},
				"models": [
					{"clientModel": "deepseek-chat", "upstreamModel": "deepseek-chat"},
					{"clientModel": "gpt-5.4-mini", "upstreamModel": "deepseek-v4-flash-vision-exp"}
				]
			}]
		}
	}]
}`

func TestAutomaticRoutingListsOnlyRecommendedGptModels(t *testing.T) {
	m := loadAutomaticRoutingManifest(t, trimmedAutomaticRoutingManifestPayload)
	spec := m.apiKeyByValue["client-key"]
	models := visibleModelsForAPIKey(m, spec)
	listed := make(map[string]bool, len(models))
	for _, model := range models {
		listed[strings.ToLower(model)] = true
	}

	for _, expected := range []string{"gpt-6-astra", "gpt-6-sol", "gpt-6-luna", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5"} {
		if !listed[expected] {
			t.Fatalf("推荐模型 %s 必须出现在模型列表里: %#v", expected, models)
		}
	}
	for _, hidden := range []string{"gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex"} {
		if listed[hidden] {
			t.Fatalf("历史模型 %s 不应出现在模型列表里: %#v", hidden, models)
		}
	}
	if !listed["deepseek-chat"] {
		t.Fatalf("供应商模型必须保留: %#v", models)
	}
}

const gptRelayAutomaticRoutingManifestPayload = `{
	"modelIds": ["gpt-5.5", "grok-4.6"],
	"accounts": [
		{"id": "gpt-relay", "email": "relay@example.com", "upstreamApiKey": "sk-relay"},
		{"id": "deepseek-relay", "email": "deepseek@example.com", "upstreamApiKey": "sk-deepseek"}
	],
	"apiKeys": [{
		"id": "client",
		"label": "Client",
		"key": "client-key",
		"enabled": true,
		"accountIds": ["gpt-relay", "deepseek-relay"],
		"allowedModels": [],
		"excludedModels": [],
		"modelRouting": {
			"automatic": true,
			"nativeModels": ["grok-4.6", "gpt-reserve"],
			"routableModels": ["gpt-5.4-mini"],
			"defaultRoute": "oauth",
			"failurePolicy": "strict",
			"routes": [{
				"id": "auto-gpt-relay",
				"namespace": "api-gpt-relay",
				"providerAccountId": "gpt-relay",
				"providerGateway": {
					"baseUrl": "http://127.0.0.1:1/v1",
					"apiKey": "sk-relay",
					"upstreamModel": "gpt-5.5",
					"upstreamModels": ["gpt-5.5"],
					"wireApi": "chat_completions"
				},
				"models": [
					{"clientModel": "gpt-5.5", "upstreamModel": "gpt-5.5"}
				]
			}, {
				"id": "auto-deepseek-relay",
				"namespace": "api-deepseek-relay",
				"providerAccountId": "deepseek-relay",
				"providerGateway": {
					"baseUrl": "http://127.0.0.1:1/v1",
					"apiKey": "sk-deepseek",
					"upstreamModel": "deepseek-flash",
					"upstreamModels": ["deepseek-flash"],
					"wireApi": "responses"
				},
				"models": [
					{"clientModel": "gpt-5.6-luna", "upstreamModel": "deepseek-flash"}
				]
			}]
		}
	}]
}`

// 第三方 GPT 中转账号（客户端名与上游名都是 GPT 家族）的模型要能出现在列表里并通过请求校验；
// 壳位别名（客户端名是 GPT、上游是 deepseek-*）仍然只保留官方推荐集。
func TestAutomaticRoutingListsGptRelayAccountModels(t *testing.T) {
	m := loadAutomaticRoutingManifest(t, gptRelayAutomaticRoutingManifestPayload)
	spec := m.apiKeyByValue["client-key"]
	models := visibleModelsForAPIKey(m, spec)
	listed := make(map[string]bool, len(models))
	for _, model := range models {
		listed[strings.ToLower(model)] = true
	}

	if !listed["gpt-5.5"] {
		t.Fatalf("GPT 中转账号自己的模型必须保留: %#v", models)
	}
	if listed["gpt-5.6-luna"] {
		t.Fatalf("壳位别名不应出现在模型列表里: %#v", models)
	}
	if !automaticClientModelVisible(m, spec, "gpt-5.5") {
		t.Fatal("GPT 中转账号的模型必须通过请求校验")
	}
	if automaticClientModelVisible(m, spec, "gpt-5.6-luna") {
		t.Fatal("壳位别名不应通过请求校验")
	}

	server := &relayServer{manifest: m}
	candidates := server.automaticCandidates(spec, "gpt-5.5")
	if len(candidates) != 1 || candidates[0].route.ProviderAccountID != "gpt-relay" {
		t.Fatalf("GPT 中转模型必须路由到对应账号: %#v", candidates)
	}
}

func TestAutomaticRoutingStillAcceptsRoutableHistoryModels(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := loadAutomaticRoutingManifest(t, trimmedAutomaticRoutingManifestPayload)
	spec := m.apiKeyByValue["client-key"]

	for _, model := range []string{"gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex"} {
		if !automaticClientModelVisible(m, spec, model) {
			t.Fatalf("历史模型 %s 必须保持可路由", model)
		}
		if !automaticNativeModel(spec, model) {
			t.Fatalf("历史模型 %s 必须按原生模型路由", model)
		}
	}
	if automaticClientModelVisible(m, spec, "gpt-5.4-unknown") {
		t.Fatal("不在清单里的模型必须被拒绝")
	}

	runtime := &fakeRuntime{response: cliproxyexecutor.Response{Payload: []byte(`{"id":"native-upstream"}`)}}
	policy := &requestPolicy{manifest: m, tracker: newRequestUsageTracker()}
	server := &relayServer{
		runtime:           runtime,
		cfg:               &config.Config{},
		manifest:          m,
		policy:            policy,
		automaticSelector: &firstAuthSelector{},
	}
	recorder := postAutomaticRoutingRequest(t, server.router(), "gpt-5.4")
	if recorder.Code != http.StatusOK {
		t.Fatalf("历史模型请求必须成功: status=%d body=%s", recorder.Code, recorder.Body.String())
	}
	if runtime.executeCalls != 1 {
		t.Fatalf("历史模型应按原生执行器处理: %#v", runtime)
	}
}

func TestAutomaticRoutingUsesOfficialDisplayNames(t *testing.T) {
	for model, want := range map[string]string{
		"gpt-6-astra":   "GPT-6 Astra",
		"gpt-6-sol":     "GPT-6 Sol",
		"gpt-6-luna":    "GPT-6 Luna",
		"gpt-5.6-sol":   "GPT-5.6 Sol",
		"gpt-5.6-terra": "GPT-5.6 Terra",
		"gpt-5.6-luna":  "GPT-5.6 Luna",
		"gpt-5.5":       "GPT-5.5",
	} {
		if got := displayNameForModel(model); got != want {
			t.Fatalf("displayNameForModel(%q) = %q, want %q", model, got, want)
		}
	}
}

// DeepSeek 账号与网关模式一致：只列出账号模型，声明可发送图片，并把图片自动转到识图模型。
const deepseekAutomaticRoutingManifestPayload = `{
	"modelIds": ["gpt-5.5"],
	"accounts": [
		{"id": "deepseek-account", "email": "deepseek@example.com", "upstreamApiKey": "sk-deepseek"}
	],
	"apiKeys": [{
		"id": "client",
		"label": "Client",
		"key": "client-key",
		"enabled": true,
		"accountIds": ["deepseek-account"],
		"allowedModels": [],
		"excludedModels": [],
		"modelRouting": {
			"automatic": true,
			"nativeModels": ["gpt-6-astra", "gpt-6-sol", "gpt-6-luna", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5", "gpt-reserve"],
			"routableModels": ["gpt-5.4", "gpt-5.4-mini"],
			"defaultRoute": "oauth",
			"failurePolicy": "strict",
			"routes": [{
				"id": "auto-deepseek",
				"namespace": "api-deepseek",
				"providerAccountId": "deepseek-account",
				"providerGateway": {
					"baseUrl": "http://%UPSTREAM%/",
					"apiKey": "sk-deepseek",
					"upstreamModel": "deepseek-flash",
					"upstreamModels": ["deepseek-flash", "deepseek-v4-pro"],
					"wireApi": "responses",
					"modelCapabilities": {
						"deepseek-flash": {"supportsVision": true},
						"deepseek-v4-pro": {"supportsVision": false}
					},
					"visionRoutingModel": "deepseek-flash"
				},
				"models": [
					{
						"clientModel": "deepseek-flash", "upstreamModel": "deepseek-flash",
						"displayName": "DeepSeek-V4.1-Flash",
						"reasoningLevels": [
							{"effort": "low", "description": "Fast responses with lighter reasoning"},
							{"effort": "high", "description": "Greater reasoning depth for complex problems"},
							{"effort": "max", "description": "Maximum reasoning depth for the hardest problems"}
						],
						"defaultReasoningLevel": "max"
					},
					{
						"clientModel": "deepseek-v4-pro", "upstreamModel": "deepseek-v4-pro",
						"displayName": "DeepSeek-V4-Pro",
						"reasoningLevels": [
							{"effort": "low", "description": "Fast responses with lighter reasoning"},
							{"effort": "high", "description": "Greater reasoning depth for complex problems"},
							{"effort": "max", "description": "Maximum reasoning depth for the hardest problems"}
						],
						"defaultReasoningLevel": "max"
					}
				]
			}]
		}
	}]
}`

func deepseekAutomaticRoutingManifest(t *testing.T, upstreamURL string) *manifest {
	t.Helper()
	payload := strings.Replace(deepseekAutomaticRoutingManifestPayload, "http://%UPSTREAM%/", upstreamURL, 1)
	return loadAutomaticRoutingManifest(t, payload)
}

func TestAutomaticRoutingListsOnlyDeepseekCatalogModels(t *testing.T) {
	m := deepseekAutomaticRoutingManifest(t, "http://127.0.0.1:1")
	spec := m.apiKeyByValue["client-key"]
	models := visibleModelsForAPIKey(m, spec)

	listed := make([]string, 0, len(models))
	for _, model := range models {
		if strings.HasPrefix(strings.ToLower(model), "deepseek") {
			listed = append(listed, model)
		}
	}
	if !reflect.DeepEqual(listed, []string{"deepseek-flash", "deepseek-v4-pro"}) {
		t.Fatalf("DeepSeek 可见模型 = %#v, want 账号模型列表里的两个模型: %#v", listed, models)
	}
}

func TestAutomaticRoutingAdvertisesImageSupportForRoutedModels(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := deepseekAutomaticRoutingManifest(t, "http://127.0.0.1:1")
	server := &relayServer{
		manifest: m,
		cfg:      &config.Config{},
		policy:   &requestPolicy{manifest: m, tracker: newRequestUsageTracker()},
	}

	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodGet, "/v1/models?client_version=1", nil)
	request.Header.Set("Authorization", "Bearer client-key")
	server.router().ServeHTTP(recorder, request)
	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", recorder.Code, recorder.Body.String())
	}
	var payload struct {
		Models []map[string]any `json:"models"`
	}
	if err := json.Unmarshal(recorder.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode models response: %v", err)
	}
	found := map[string]bool{}
	for _, model := range payload.Models {
		slug, _ := model["slug"].(string)
		if !strings.HasPrefix(slug, "deepseek") {
			continue
		}
		modalities, _ := model["input_modalities"].([]any)
		for _, modality := range modalities {
			if modality == "image" {
				found[slug] = true
			}
		}
	}
	for _, slug := range []string{"deepseek-flash", "deepseek-v4-pro"} {
		if !found[slug] {
			t.Fatalf("%s 必须声明可发送图片（图片会由网关转到识图模型）: %#v", slug, payload.Models)
		}
	}
}

// 客户端目录必须与 DeepSeek 网关模式一致：官方显示名 + 三档推理（含最高档）。
func modelSlugs(models []map[string]any) []string {
	slugs := make([]string, 0, len(models))
	for _, model := range models {
		if slug, ok := model["slug"].(string); ok {
			slugs = append(slugs, slug)
		}
	}
	return slugs
}

func TestAutomaticRoutingPublishesRouteModelNamesAndReasoningLevels(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := deepseekAutomaticRoutingManifest(t, "http://127.0.0.1:1")
	server := &relayServer{
		manifest: m,
		cfg:      &config.Config{},
		policy:   &requestPolicy{manifest: m, tracker: newRequestUsageTracker()},
	}

	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodGet, "/v1/models?client_version=1", nil)
	request.Header.Set("Authorization", "Bearer client-key")
	server.router().ServeHTTP(recorder, request)
	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", recorder.Code, recorder.Body.String())
	}
	var payload struct {
		Models []map[string]any `json:"models"`
	}
	if err := json.Unmarshal(recorder.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode models response: %v", err)
	}

	bySlug := map[string]map[string]any{}
	for _, model := range payload.Models {
		if slug, ok := model["slug"].(string); ok {
			bySlug[slug] = model
		}
	}
	for slug, wantName := range map[string]string{
		"gpt-6-astra":   "GPT-6 Astra",
		"gpt-6-sol":     "GPT-6 Sol",
		"gpt-6-luna":    "GPT-6 Luna",
		"gpt-5.6-sol":   "GPT-5.6 Sol",
		"gpt-5.6-terra": "GPT-5.6 Terra",
		"gpt-5.6-luna":  "GPT-5.6 Luna",
		"gpt-5.5":       "GPT-5.5",
		"gpt-reserve":   "GPT-5.6 Reserve",
	} {
		model := bySlug[slug]
		if model == nil {
			t.Fatalf("模型 %s 缺失，实际: %v", slug, modelSlugs(payload.Models))
		}
		if got := stringFromAny(model["display_name"]); got != wantName {
			t.Fatalf("%s 显示名 = %q, want %q", slug, got, wantName)
		}
	}

	for _, slug := range []string{"deepseek-flash", "deepseek-v4-pro"} {
		model := bySlug[slug]
		if model == nil {
			t.Fatalf("模型 %s 缺失，实际: %v", slug, modelSlugs(payload.Models))
		}
		levels, _ := model["supported_reasoning_levels"].([]any)
		efforts := make([]string, 0, len(levels))
		for _, level := range levels {
			if entry, ok := level.(map[string]any); ok {
				efforts = append(efforts, stringFromAny(entry["effort"]))
			}
		}
		if !reflect.DeepEqual(efforts, []string{"low", "high", "max"}) {
			t.Fatalf("%s 推理档位 = %#v, want low/high/max（含最高档）", slug, efforts)
		}
		if got := stringFromAny(model["default_reasoning_level"]); got != "max" {
			t.Fatalf("%s 默认档位 = %q, want max（DeepSeek 默认最高档）", slug, got)
		}
	}
}

func TestAutomaticRoutingSendsImagesToVisionModel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var receivedBody []byte
	var receivedModel string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		receivedBody = body
		var payload struct {
			Model string `json:"model"`
		}
		_ = json.Unmarshal(body, &payload)
		receivedModel = payload.Model
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"deepseek-upstream"}`))
	}))
	defer upstream.Close()

	m := deepseekAutomaticRoutingManifest(t, upstream.URL)
	server := &relayServer{
		manifest: m,
		cfg:      &config.Config{},
		policy:   &requestPolicy{manifest: m, tracker: newRequestUsageTracker()},
	}
	requestBody := `{"model":"deepseek-v4-pro","input":[{"role":"user","content":[{"type":"input_text","text":"描述这张图"},{"type":"input_image","image_url":"data:image/png;base64,AAAA"}]}]}`
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(requestBody))
	request.Header.Set("Authorization", "Bearer client-key")
	request.Header.Set("Content-Type", "application/json")
	recorder := httptest.NewRecorder()
	server.router().ServeHTTP(recorder, request)

	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", recorder.Code, recorder.Body.String())
	}
	if receivedModel != "deepseek-flash" {
		t.Fatalf("带图片的 deepseek-v4-pro 请求必须转到识图模型，上游收到 model=%q body=%s", receivedModel, receivedBody)
	}
}

func TestAutomaticRoutingKeepsTextOnlyModelForTextRequests(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var receivedModel string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		var payload struct {
			Model string `json:"model"`
		}
		_ = json.Unmarshal(body, &payload)
		receivedModel = payload.Model
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"deepseek-upstream"}`))
	}))
	defer upstream.Close()

	m := deepseekAutomaticRoutingManifest(t, upstream.URL)
	server := &relayServer{
		manifest: m,
		cfg:      &config.Config{},
		policy:   &requestPolicy{manifest: m, tracker: newRequestUsageTracker()},
	}
	requestBody := `{"model":"deepseek-v4-pro","input":[{"role":"user","content":[{"type":"input_text","text":"你好"}]}]}`
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(requestBody))
	request.Header.Set("Authorization", "Bearer client-key")
	request.Header.Set("Content-Type", "application/json")
	recorder := httptest.NewRecorder()
	server.router().ServeHTTP(recorder, request)

	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", recorder.Code, recorder.Body.String())
	}
	if receivedModel != "deepseek-v4-pro" {
		t.Fatalf("纯文本请求不应改用识图模型，上游收到 model=%q", receivedModel)
	}
}
