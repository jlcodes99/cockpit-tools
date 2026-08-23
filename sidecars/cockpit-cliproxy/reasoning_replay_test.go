package main

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/tidwall/gjson"
)

func TestProviderGatewayShouldReplayReasoning(t *testing.T) {
	t.Parallel()

	if !providerGatewayShouldReplayReasoning(&providerGatewaySpec{
		BaseURL:       "https://api.deepseek.com",
		UpstreamModel: "deepseek-v4-flash",
		WireAPI:       "chat_completions",
	}, "deepseek-v4-flash") {
		t.Fatal("deepseek-v4-flash should replay reasoning")
	}
	if !providerGatewayShouldReplayReasoning(&providerGatewaySpec{
		BaseURL:       "https://api.deepseek.com/v1",
		UpstreamModel: "deepseek-v4-pro",
	}, "gpt-5.4") {
		t.Fatal("aliased DeepSeek upstream should still replay reasoning")
	}
	if providerGatewayShouldReplayReasoning(&providerGatewaySpec{
		BaseURL:       "https://open.bigmodel.cn/api/paas/v4",
		UpstreamModel: "glm-5.1",
	}, "glm-5.1") {
		t.Fatal("non-DeepSeek models must not inject reasoning_content")
	}
	if providerGatewayShouldReplayReasoning(&providerGatewaySpec{
		BaseURL:         "https://api.deepseek.com",
		UpstreamModel:   "deepseek-v4-flash",
		DisableThinking: true,
	}, "deepseek-v4-flash") {
		t.Fatal("disableThinking should skip reasoning replay")
	}
	if providerGatewayShouldReplayReasoning(&providerGatewaySpec{
		BaseURL:       "https://api.deepseek.com",
		UpstreamModel: "deepseek-reasoner",
	}, "deepseek-reasoner") {
		t.Fatal("legacy deepseek-reasoner should not replay")
	}
}

func TestReplayReasoningIntoChatCompletionsUsesToolCallCache(t *testing.T) {
	clearReasoningReplayCache()
	t.Cleanup(clearReasoningReplayCache)

	model := "deepseek-v4-flash"
	cacheReasoningFromChatCompletionsResponse(model, []byte(`{
		"choices":[{
			"message":{
				"role":"assistant",
				"content":"",
				"reasoning_content":"need to write a.txt",
				"tool_calls":[{"id":"call_abc","type":"function","function":{"name":"shell","arguments":"{}"}}]
			}
		}]
	}`))

	out := replayReasoningIntoChatCompletions(model, []byte(`{
		"model":"deepseek-v4-flash",
		"messages":[
			{"role":"user","content":"create a.txt"},
			{"role":"assistant","tool_calls":[{"id":"call_abc","type":"function","function":{"name":"shell","arguments":"{}"}}],"reasoning_content":"tool call"},
			{"role":"tool","tool_call_id":"call_abc","content":"ok"}
		]
	}`))
	got := gjson.GetBytes(out, "messages.1.reasoning_content").String()
	if got != "need to write a.txt" {
		t.Fatalf("reasoning_content = %q, want cached thought", got)
	}
}

func TestReplayReasoningIntoChatCompletionsKeepsRealReasoning(t *testing.T) {
	clearReasoningReplayCache()
	t.Cleanup(clearReasoningReplayCache)

	model := "deepseek-v4-pro"
	cacheReasoningFromChatCompletionsResponse(model, []byte(`{
		"choices":[{
			"message":{
				"role":"assistant",
				"content":"done",
				"reasoning_content":"cached thought",
				"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{}"}}]
			}
		}]
	}`))

	out := replayReasoningIntoChatCompletions(model, []byte(`{
		"messages":[
			{"role":"assistant","content":"done","reasoning_content":"client thought","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{}"}}]}
		]
	}`))
	got := gjson.GetBytes(out, "messages.0.reasoning_content").String()
	if got != "client thought" {
		t.Fatalf("reasoning_content = %q, want client thought", got)
	}
}

func TestReplayReasoningIntoChatCompletionsUsesContentFingerprint(t *testing.T) {
	clearReasoningReplayCache()
	t.Cleanup(clearReasoningReplayCache)

	model := "deepseek-v4-pro"
	cacheReasoningFromChatCompletionsResponse(model, []byte(`{
		"choices":[{
			"message":{
				"role":"assistant",
				"content":"the file says hello",
				"reasoning_content":"final thought after tools"
			}
		}]
	}`))

	out := replayReasoningIntoChatCompletions(model, []byte(`{
		"messages":[
			{"role":"assistant","content":"the file says hello"}
		]
	}`))
	got := gjson.GetBytes(out, "messages.0.reasoning_content").String()
	if got != "final thought after tools" {
		t.Fatalf("reasoning_content = %q, want fingerprint replay", got)
	}
}

func TestCacheReasoningIgnoresPlaceholder(t *testing.T) {
	clearReasoningReplayCache()
	t.Cleanup(clearReasoningReplayCache)

	cacheReasoningFromChatCompletionsMessages("deepseek-v4-flash", []byte(`{
		"messages":[
			{"role":"assistant","reasoning_content":"tool call","tool_calls":[{"id":"call_skip","type":"function","function":{"name":"shell","arguments":"{}"}}]}
		]
	}`))
	if _, ok := getReasoningReplay(reasoningReplayToolCallKey("call_skip"), "deepseek-v4-flash"); ok {
		t.Fatal("placeholder reasoning must not be cached")
	}
}

func TestDisableChatCompletionsThinking(t *testing.T) {
	out := disableChatCompletionsThinking([]byte(`{
		"model":"deepseek-v4-flash",
		"reasoning_effort":"high",
		"reasoning":{"effort":"high"},
		"messages":[
			{"role":"assistant","content":"hi","reasoning_content":"thought"}
		]
	}`))
	if gjson.GetBytes(out, "thinking.type").String() != "disabled" {
		t.Fatalf("thinking.type = %q, want disabled; out=%s", gjson.GetBytes(out, "thinking.type").String(), out)
	}
	if gjson.GetBytes(out, "reasoning_effort").Exists() {
		t.Fatalf("reasoning_effort should be removed; out=%s", out)
	}
	if gjson.GetBytes(out, "messages.0.reasoning_content").Exists() {
		t.Fatalf("historical reasoning_content should be stripped when thinking is disabled; out=%s", out)
	}
}

func TestChatStreamReasoningAccumulatorCachesToolCall(t *testing.T) {
	clearReasoningReplayCache()
	t.Cleanup(clearReasoningReplayCache)

	acc := newChatStreamReasoningAccumulator()
	acc.consume([]byte(`data: {"choices":[{"delta":{"role":"assistant","reasoning_content":"plan "}}]}`))
	acc.consume([]byte(`data: {"choices":[{"delta":{"reasoning_content":"to call shell","tool_calls":[{"index":0,"id":"call_stream","type":"function","function":{"name":"shell","arguments":""}}]}}]}`))
	acc.consume([]byte(`data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}}]}`))
	acc.consume([]byte(`data: [DONE]`))
	acc.cache("deepseek-v4-flash")

	got, ok := getReasoningReplay(reasoningReplayToolCallKey("call_stream"), "deepseek-v4-flash")
	if !ok {
		t.Fatal("expected streamed reasoning to be cached")
	}
	if got != "plan to call shell" {
		t.Fatalf("cached reasoning = %q, want concatenated stream", got)
	}
}

func TestPrepareProviderGatewayChatCompletionsReasoningDisableThinking(t *testing.T) {
	gateway := &providerGatewaySpec{
		BaseURL:         "https://api.deepseek.com",
		UpstreamModel:   "deepseek-v4-flash",
		DisableThinking: true,
	}
	out := prepareProviderGatewayChatCompletionsReasoning(gateway, "deepseek-v4-flash", []byte(`{
		"model":"deepseek-v4-flash",
		"reasoning_effort":"medium",
		"messages":[{"role":"user","content":"hi"}]
	}`))
	if gjson.GetBytes(out, "thinking.type").String() != "disabled" {
		t.Fatalf("expected thinking disabled, out=%s", out)
	}
}

func TestRelayServerProviderGatewayReplaysDeepSeekReasoningOnToolFollowUp(t *testing.T) {
	gin.SetMode(gin.TestMode)
	clearReasoningReplayCache()
	t.Cleanup(clearReasoningReplayCache)

	var upstreamBodies []string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		upstreamBodies = append(upstreamBodies, string(body))
		w.Header().Set("Content-Type", "application/json")
		if strings.Contains(string(body), `"role":"tool"`) {
			_, _ = w.Write([]byte(`{"id":"chatcmpl_2","object":"chat.completion","created":2,"model":"deepseek-v4-flash","choices":[{"index":0,"message":{"role":"assistant","content":"created a.txt","reasoning_content":"file is ready"},"finish_reason":"stop"}]}`))
			return
		}
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"deepseek-v4-flash","choices":[{"index":0,"message":{"role":"assistant","content":"","reasoning_content":"I should write a.txt","tool_calls":[{"id":"call_abc","type":"function","function":{"name":"shell","arguments":"{\"command\":\"echo 123\"}"}}]},"finish_reason":"tool_calls"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "deepseek-key",
		UpstreamModel:  "deepseek-v4-flash",
		UpstreamModels: []string{"deepseek-v4-flash"},
		WireAPI:        "chat_completions",
	}
	router := newProviderGatewayTestRouter(gateway)

	first := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{
		"model":"deepseek-v4-flash",
		"input":[{"type":"message","role":"user","content":"create a.txt"}],
		"stream":false
	}`))
	first.Header.Set("Authorization", "Bearer client-key")
	first.Header.Set("Content-Type", "application/json")
	firstW := httptest.NewRecorder()
	router.ServeHTTP(firstW, first)
	if firstW.Code != http.StatusOK {
		t.Fatalf("first turn status=%d body=%s", firstW.Code, firstW.Body.String())
	}

	second := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{
		"model":"deepseek-v4-flash",
		"input":[
			{"type":"message","role":"user","content":"create a.txt"},
			{"type":"function_call","call_id":"call_abc","name":"shell","arguments":"{\"command\":\"echo 123\"}"},
			{"type":"function_call_output","call_id":"call_abc","output":"ok"}
		],
		"stream":false
	}`))
	second.Header.Set("Authorization", "Bearer client-key")
	second.Header.Set("Content-Type", "application/json")
	secondW := httptest.NewRecorder()
	router.ServeHTTP(secondW, second)
	if secondW.Code != http.StatusOK {
		t.Fatalf("second turn status=%d body=%s", secondW.Code, secondW.Body.String())
	}
	if len(upstreamBodies) < 2 {
		t.Fatalf("expected two upstream requests, got %d", len(upstreamBodies))
	}
	got := gjson.Get(upstreamBodies[1], "messages.#(role=assistant).reasoning_content").String()
	if got != "I should write a.txt" {
		t.Fatalf("follow-up reasoning_content = %q, want cached thought; body=%s", got, upstreamBodies[1])
	}
	if strings.Contains(upstreamBodies[1], `"reasoning_content":"tool call"`) {
		t.Fatalf("placeholder must not be sent upstream: %s", upstreamBodies[1])
	}
}

func TestRelayServerProviderGatewayReplaysDeepSeekReasoningFromStream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	clearReasoningReplayCache()
	t.Cleanup(clearReasoningReplayCache)

	var followUpBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		if strings.Contains(string(body), `"role":"tool"`) {
			followUpBody = string(body)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"id":"chatcmpl_2","object":"chat.completion","created":2,"model":"deepseek-v4-flash","choices":[{"index":0,"message":{"role":"assistant","content":"done","reasoning_content":"wrap up"},"finish_reason":"stop"}]}`))
			return
		}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "data: {\"id\":\"chatcmpl_1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"deepseek-v4-flash\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"need a file\"},\"finish_reason\":null}]}\n\n")
		_, _ = io.WriteString(w, "data: {\"id\":\"chatcmpl_1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"deepseek-v4-flash\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_stream\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\n")
		_, _ = io.WriteString(w, "data: {\"id\":\"chatcmpl_1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"deepseek-v4-flash\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n")
		_, _ = io.WriteString(w, "data: [DONE]\n\n")
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "deepseek-key",
		UpstreamModel:  "deepseek-v4-flash",
		UpstreamModels: []string{"deepseek-v4-flash"},
		WireAPI:        "chat_completions",
	}
	router := newProviderGatewayTestRouter(gateway)

	first := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{
		"model":"deepseek-v4-flash",
		"input":[{"type":"message","role":"user","content":"create a.txt"}],
		"stream":true
	}`))
	first.Header.Set("Authorization", "Bearer client-key")
	first.Header.Set("Content-Type", "application/json")
	firstW := httptest.NewRecorder()
	router.ServeHTTP(firstW, first)
	if firstW.Code != http.StatusOK {
		t.Fatalf("first turn status=%d body=%s", firstW.Code, firstW.Body.String())
	}

	second := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{
		"model":"deepseek-v4-flash",
		"input":[
			{"type":"message","role":"user","content":"create a.txt"},
			{"type":"function_call","call_id":"call_stream","name":"shell","arguments":"{}"},
			{"type":"function_call_output","call_id":"call_stream","output":"ok"}
		],
		"stream":false
	}`))
	second.Header.Set("Authorization", "Bearer client-key")
	second.Header.Set("Content-Type", "application/json")
	secondW := httptest.NewRecorder()
	router.ServeHTTP(secondW, second)
	if secondW.Code != http.StatusOK {
		t.Fatalf("second turn status=%d body=%s", secondW.Code, secondW.Body.String())
	}
	got := gjson.Get(followUpBody, "messages.#(role=assistant).reasoning_content").String()
	if got != "need a file" {
		t.Fatalf("streamed reasoning was not replayed: %q body=%s", got, followUpBody)
	}
}

func TestRelayServerProviderGatewayDisableThinkingStripsReasoningEffort(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var upstreamBody string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		upstreamBody = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl_1","object":"chat.completion","created":1,"model":"deepseek-v4-flash","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:         upstream.URL,
		APIKey:          "deepseek-key",
		UpstreamModel:   "deepseek-v4-flash",
		UpstreamModels:  []string{"deepseek-v4-flash"},
		WireAPI:         "chat_completions",
		DisableThinking: true,
	}
	router := newProviderGatewayTestRouter(gateway)

	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{
		"model":"deepseek-v4-flash",
		"reasoning":{"effort":"high"},
		"input":"hello",
		"stream":false
	}`))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", w.Code, w.Body.String())
	}
	if gjson.Get(upstreamBody, "thinking.type").String() != "disabled" {
		t.Fatalf("expected thinking disabled, body=%s", upstreamBody)
	}
	if gjson.Get(upstreamBody, "reasoning_effort").Exists() {
		t.Fatalf("reasoning_effort should be stripped, body=%s", upstreamBody)
	}
}

func newProviderGatewayTestRouter(gateway *providerGatewaySpec) http.Handler {
	m := &manifest{
		APIKeys:  []apiKeySpec{{ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway}},
		ModelIDs: []string{gateway.UpstreamModel},
		apiKeyByValue: map[string]*apiKeySpec{
			"client-key": {ID: "provider_gateway_account_1", Label: "Provider Gateway", Key: "client-key", Enabled: true, ProviderGateway: gateway},
		},
	}
	return (&relayServer{
		runtime:  &fakeRuntime{},
		cfg:      &config.Config{},
		manifest: m,
		policy:   &requestPolicy{manifest: m},
	}).router()
}

func TestAssistantReasoningFingerprintStableForTextParts(t *testing.T) {
	stringMsg := gjson.Parse(`{"role":"assistant","content":"hello"}`)
	partsMsg := gjson.Parse(`{"role":"assistant","content":[{"type":"text","text":"hello"}]}`)
	if assistantReasoningFingerprint(stringMsg) != assistantReasoningFingerprint(partsMsg) {
		t.Fatal("content fingerprint should treat string and text parts as equivalent")
	}
}
