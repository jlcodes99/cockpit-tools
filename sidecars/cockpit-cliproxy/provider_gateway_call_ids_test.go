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

func TestNormalizeProviderGatewayCallIDsRepairsMissingReplayIDs(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-flash","input":[
		{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"pwd\"}"},
		{"type":"function_call_output","output":"/workspace"},
		{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch"},
		{"type":"custom_tool_call_output","output":"Done!"},
		{"type":"function_call","call_id":"call_existing","name":"lookup","arguments":"{}"},
		{"type":"function_call_output","call_id":"call_existing","output":"ok"}
	]}`)

	got := normalizeProviderGatewayCallIDs(body)
	firstCallID := gjson.GetBytes(got, "input.0.call_id").String()
	customCallID := gjson.GetBytes(got, "input.2.call_id").String()
	if !strings.HasPrefix(firstCallID, "call_missing") {
		t.Fatalf("missing function call id was not synthesized: %s", got)
	}
	if gotID := gjson.GetBytes(got, "input.1.call_id").String(); gotID != firstCallID {
		t.Fatalf("function_call_output call_id = %q, want %q", gotID, firstCallID)
	}
	if !strings.HasPrefix(customCallID, "call_missing") {
		t.Fatalf("missing custom tool call id was not synthesized: %s", got)
	}
	if gotID := gjson.GetBytes(got, "input.3.call_id").String(); gotID != customCallID {
		t.Fatalf("custom_tool_call_output call_id = %q, want %q", gotID, customCallID)
	}
	if gotID := gjson.GetBytes(got, "input.4.call_id").String(); gotID != "call_existing" {
		t.Fatalf("existing call_id changed to %q", gotID)
	}
}

func TestNormalizeProviderGatewayCallIDsDropsAnonymousOrphanOutputs(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-flash","input":[
		{"type":"message","role":"user","content":"continue"},
		{"type":"function_call_output","output":"orphan result"},
		{"type":"function_call","name":"exec_command","arguments":"{}"},
		{"type":"function_call_output","output":"paired result"},
		{"type":"function_call_output","name":"heartbeat","output":"keep standalone"}
	]}`)

	got := normalizeProviderGatewayCallIDs(body)
	if strings.Contains(string(got), "call_missing_output") {
		t.Fatalf("anonymous orphan output received synthetic call_id: %s", got)
	}
	input := gjson.GetBytes(got, "input").Array()
	if len(input) != 4 {
		t.Fatalf("input len = %d, want 4: %s", len(input), got)
	}
	callID := input[1].Get("call_id").String()
	if callID == "" || input[2].Get("call_id").String() != callID {
		t.Fatalf("paired call/output IDs were not repaired together: %s", got)
	}
	if input[3].Get("name").String() != "heartbeat" || input[3].Get("call_id").Exists() {
		t.Fatalf("standalone named function_call_output was not preserved: %s", got)
	}
}

func TestProviderGatewayCallIDRepairUnblocksToolOrder(t *testing.T) {
	body := toolOrderBody(
		`{"type":"message","role":"user","content":"run pwd"}`,
		`{"type":"function_call","name":"exec_command","arguments":"{}"}`,
		`{"type":"message","role":"developer","content":"hook note"}`,
		`{"type":"function_call_output","output":"/tmp"}`,
	)
	if _, _, ok := providerGatewayRepairsToolCallOrderBody(body); ok {
		t.Fatal("missing call_id must still abort reorder before repair")
	}

	repaired := normalizeProviderGatewayCallIDs(body)
	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(repaired)
	if !ok {
		t.Fatal("repaired call_id must allow reorder")
	}
	if relocated != 1 {
		t.Fatalf("relocated = %d, want 1: %s", relocated, got)
	}
	callID := gjson.GetBytes(got, "input.1.call_id").String()
	if !strings.HasPrefix(callID, "call_missing") {
		t.Fatalf("synthesized call_id missing after reorder: %s", got)
	}
	want := []string{
		"message:",
		"function_call:" + callID,
		"function_call_output:" + callID,
		"message:",
	}
	if gotTypes := toolOrderTypes(got); strings.Join(gotTypes, ",") != strings.Join(want, ",") {
		t.Fatalf("order = %v, want %v from %s", gotTypes, want, got)
	}
}

func TestProviderGatewayForwardsRepairedCallIDsToDeepSeek(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var forwarded string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		forwarded = string(body)
		for _, item := range gjson.GetBytes(body, "input").Array() {
			itemType := item.Get("type").String()
			if itemType == "function_call" || itemType == "function_call_output" {
				if strings.TrimSpace(item.Get("call_id").String()) == "" {
					w.Header().Set("Content-Type", "application/json")
					w.WriteHeader(http.StatusUnprocessableEntity)
					_, _ = w.Write([]byte(`{"error":{"message":"missing field call_id","type":"invalid_request_error"}}`))
					return
				}
			}
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"resp-test","object":"response","status":"completed","output":[]}`))
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
	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"deepseek-v4-flash","input":[{"type":"function_call","name":"exec_command","arguments":"{}"},{"type":"function_call_output","output":"/tmp"},{"type":"message","role":"user","content":"continue"}],"stream":false}`))
	req.Header.Set("Authorization", "Bearer test-client")
	req.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	router.ServeHTTP(response, req)
	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s forwarded=%s", response.Code, response.Body.String(), forwarded)
	}
	callID := gjson.Get(forwarded, "input.0.call_id").String()
	if !strings.HasPrefix(callID, "call_missing") {
		t.Fatalf("upstream did not receive synthesized call_id: %s", forwarded)
	}
	if got := gjson.Get(forwarded, "input.1.call_id").String(); got != callID {
		t.Fatalf("output call_id = %q, want %q from %s", got, callID, forwarded)
	}
}

func TestProviderGatewayCallIDRepairIsFormatGated(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var forwarded string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		forwarded = string(body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"chatcmpl-test","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`))
	}))
	defer upstream.Close()

	gateway := &providerGatewaySpec{
		BaseURL:        upstream.URL,
		APIKey:         "test-upstream-key",
		UpstreamModel:  "deepseek-chat",
		UpstreamModels: []string{"deepseek-chat"},
		WireAPI:        "chat_completions",
	}
	spec := apiKeySpec{ID: "test-key", Key: "test-client", Enabled: true, ProviderGateway: gateway}
	m := &manifest{
		APIKeys:       []apiKeySpec{spec},
		ModelIDs:      []string{"deepseek-chat"},
		apiKeyByValue: map[string]*apiKeySpec{"test-client": &spec},
	}
	router := (&relayServer{runtime: &fakeRuntime{}, cfg: &config.Config{}, manifest: m, policy: &requestPolicy{manifest: m}}).router()
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(`{"model":"deepseek-chat","messages":[{"role":"user","content":"hello"}],"stream":false}`))
	req.Header.Set("Authorization", "Bearer test-client")
	req.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	router.ServeHTTP(response, req)
	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s forwarded=%s", response.Code, response.Body.String(), forwarded)
	}
	if gjson.Get(forwarded, "input").Exists() {
		t.Fatalf("chat completions request was rewritten into responses: %s", forwarded)
	}
	if gjson.Get(forwarded, "messages.0.content").String() != "hello" {
		t.Fatalf("chat completions body was damaged: %s", forwarded)
	}
}
