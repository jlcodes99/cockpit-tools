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

func TestRestoreProviderGatewayMultiAgentV2SSELine(t *testing.T) {
	line := []byte("data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"name\":\"collaboration-optimize__send_message\"}}\n")
	restored := string(restoreProviderGatewayMultiAgentV2SSELine(line, true))
	if !strings.Contains(restored, `"name":"send_message"`) || !strings.Contains(restored, `"namespace":"collaboration"`) || strings.Contains(restored, "collaboration-optimize") {
		t.Fatalf("optimized namespace was not restored: %s", restored)
	}
	ordinaryLine := []byte("data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"name\":\"exec_command\"}}\n")
	if got := string(restoreProviderGatewayMultiAgentV2SSELine(ordinaryLine, false)); got != string(ordinaryLine) {
		t.Fatalf("ordinary tool line must stay unchanged: %s", got)
	}
	eventLine := []byte("event: response.output_item.done\n")
	if got := string(restoreProviderGatewayMultiAgentV2SSELine(eventLine, true)); got != string(eventLine) {
		t.Fatalf("event line must stay unchanged: %s", got)
	}
	flatLine := []byte("data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"name\":\"collaboration::spawn_agent\"}}\n")
	flatRestored := string(restoreProviderGatewayMultiAgentV2SSELine(flatLine, false))
	if !strings.Contains(flatRestored, `"name":"spawn_agent"`) || !strings.Contains(flatRestored, `"namespace":"collaboration"`) {
		t.Fatalf("flat collaboration tool name was not normalized: %s", flatRestored)
	}
}

func TestProviderGatewayOptimizesCodexMultiAgentV2RequestAndRestoresResponse(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var forwarded string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		forwarded = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"resp_1","object":"response","status":"completed","output":[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"collaboration-optimize__send_message","arguments":"{}"},{"type":"function_call","id":"fc_2","call_id":"call_2","name":"collaboration::spawn_agent","arguments":"{}"},{"type":"function_call","id":"fc_3","call_id":"call_3","name":"collaboration::wait_agent","arguments":"{\"timeout_ms\":180000.0}"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "test-upstream-key",
		UpstreamModel:  "deepseek-v4-flash",
		UpstreamModels: []string{"deepseek-v4-flash"},
		WireAPI:        "responses",
	}
	spec := apiKeySpec{ID: "test-key", Key: "test-client", Enabled: true, ProviderGateway: gateway}
	m := &manifest{
		APIKeys:       []apiKeySpec{spec},
		ModelIDs:      []string{"deepseek-v4-flash"},
		apiKeyByValue: map[string]*apiKeySpec{"test-client": &spec},
	}
	cfg := &config.Config{}
	cfg.Codex.OptimizeMultiAgentV2 = true
	router := (&relayServer{runtime: &fakeRuntime{}, cfg: cfg, manifest: m, policy: &requestPolicy{manifest: m}}).router()

	payload := `{"model":"deepseek-v4-flash","stream":false,"input":[{"type":"message","role":"user","content":"hi"},{"type":"agent_message","author":{"role":"user"},"recipient":"root","content":[{"type":"encrypted_content","encrypted_content":"child task text"}]}],"tools":[{"type":"namespace","name":"collaboration","description":"Tools for spawning and managing sub-agents.","tools":[{"type":"function","name":"spawn_agent","description":"Spawn","parameters":{"type":"object","properties":{}}},{"type":"function","name":"send_message","description":"Send","parameters":{"type":"object","properties":{"message":{"type":"string","encrypted":true}}}}]}],"tool_choice":"auto"}`
	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(payload))
	req.Header.Set("Authorization", "Bearer test-client")
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("User-Agent", "Codex Desktop/0.151.0")
	response := httptest.NewRecorder()
	router.ServeHTTP(response, req)
	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s forwarded=%s", response.Code, response.Body.String(), forwarded)
	}

	if got := gjson.Get(forwarded, "tools.0.name").String(); got != "collaboration-optimize" {
		t.Fatalf("forwarded collaboration namespace = %q, want collaboration-optimize; body=%s", got, forwarded)
	}
	if gjson.Get(forwarded, "tools.0.tools.1.parameters.properties.message.encrypted").Exists() {
		t.Fatalf("encrypted message parameter must be removed; body=%s", forwarded)
	}
	var agentMessage gjson.Result
	for _, item := range gjson.Get(forwarded, "input").Array() {
		if item.Get("type").String() == "message" && item.Get("content.0.text").String() == "child task text" {
			agentMessage = item
			break
		}
	}
	if !agentMessage.Exists() {
		t.Fatalf("agent_message must be converted into a portable message item; body=%s", forwarded)
	}
	if agentMessage.Get("author").Exists() || agentMessage.Get("recipient").Exists() {
		t.Fatalf("private routing fields must not reach strict Responses upstream: %s", forwarded)
	}
	if got := agentMessage.Get("content.1.text").String(); got != `Agent routing metadata: {"author":{"role":"user"},"recipient":"root"}` {
		t.Fatalf("routing metadata must be preserved in content: %q", got)
	}
	if got := agentMessage.Get("content.0.type").String(); got != "input_text" {
		t.Fatalf("agent_message content.0.type = %q, want input_text; body=%s", got, forwarded)
	}
	if got := agentMessage.Get("content.0.text").String(); got != "child task text" {
		t.Fatalf("agent_message content.0.text = %q, want the task text; body=%s", got, forwarded)
	}
	if got := gjson.Get(response.Body.String(), "output.0.name").String(); got != "send_message" {
		t.Fatalf("client response tool name = %q, want send_message; body=%s", got, response.Body.String())
	}
	if got := gjson.Get(response.Body.String(), "output.0.namespace").String(); got != "collaboration" {
		t.Fatalf("client response namespace = %q, want collaboration; body=%s", got, response.Body.String())
	}
	if got := gjson.Get(response.Body.String(), "output.1.name").String(); got != "spawn_agent" {
		t.Fatalf("flat collaboration tool name = %q, want spawn_agent; body=%s", got, response.Body.String())
	}
	if got := gjson.Get(response.Body.String(), "output.1.namespace").String(); got != "collaboration" {
		t.Fatalf("flat collaboration namespace = %q, want collaboration; body=%s", got, response.Body.String())
	}
	if got := gjson.Get(response.Body.String(), "output.2.arguments").String(); got != `{"timeout_ms":180000}` {
		t.Fatalf("wait_agent arguments = %q, want integral timeout_ms; body=%s", got, response.Body.String())
	}
}

func TestProviderGatewayProjectsWebSearchHistoryForStrictUpstream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var forwarded string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		forwarded = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"resp_1","object":"response","status":"completed","output":[]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "test-upstream-key",
		UpstreamModel:  "deepseek-v4-flash",
		UpstreamModels: []string{"deepseek-v4-flash"},
		WireAPI:        "responses",
	}
	spec := apiKeySpec{ID: "test-key", Key: "test-client", Enabled: true, ProviderGateway: gateway}
	m := &manifest{
		APIKeys:       []apiKeySpec{spec},
		ModelIDs:      []string{"deepseek-v4-flash"},
		apiKeyByValue: map[string]*apiKeySpec{"test-client": &spec},
	}
	router := (&relayServer{runtime: &fakeRuntime{}, cfg: &config.Config{}, manifest: m, policy: &requestPolicy{manifest: m}}).router()

	payload := `{"model":"deepseek-v4-flash","stream":false,"input":[{"type":"message","role":"user","content":"hi"},{"type":"web_search_call","id":"ws_1","status":"completed","action":{"type":"search","query":"codex apply_patch"}}]}`
	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(payload))
	req.Header.Set("Authorization", "Bearer test-client")
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("User-Agent", "Codex Desktop/0.155.0-alpha.9.2")
	response := httptest.NewRecorder()
	router.ServeHTTP(response, req)
	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s forwarded=%s", response.Code, response.Body.String(), forwarded)
	}
	if got := gjson.Get(forwarded, `input.#(type=="web_search_call").action.queries.0`).String(); got != "codex apply_patch" {
		t.Fatalf("web_search_call queries[0] = %q, want the original query; body=%s", got, forwarded)
	}
}
