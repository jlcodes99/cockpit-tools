package main

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/tidwall/gjson"
)

func TestProviderGatewayCodexAgentMessages(t *testing.T) {
	gin.SetMode(gin.TestMode)
	const model = "gateway-test-model"
	const baseline = `{"type":"message","role":"user","content":[{"type":"input_text","text":"baseline"}]}`
	const previousReply = `{"type":"message","role":"assistant","content":[{"type":"output_text","text":"previous reply"}]}`
	const agentTask = `{"type":"agent_message","id":"amsg_test","author":"/root","recipient":"/root/worker","content":[{"type":"input_text","text":"Message Type: NEW_TASK\nPayload:\n"},{"type":"encrypted_content","encrypted_content":"Review only the assigned passage. 校验码：任务送达"}]}`
	const toolCall = `{"type":"function_call","call_id":"call_read","name":"read_passage","arguments":"{}"}`
	const toolOutput = `{"type":"function_call_output","call_id":"call_read","output":"assigned passage"}`
	const codexUA = "codex-tui/0.153.4 (Windows; x86_64)"

	cases := []struct {
		name       string
		input      string
		userAgent  string
		enabled    bool
		wireAPI    string
		wantRoles  []string
		wantTask   bool
		wantToolID bool
	}{
		{"initial", baseline + "," + agentTask, codexUA, true, "chat_completions", []string{"user", "user"}, true, false},
		{"followup", baseline + "," + previousReply + "," + agentTask, codexUA, true, "chat_completions", []string{"user", "assistant", "user"}, true, false},
		{"after_tool_result", baseline + "," + toolCall + "," + toolOutput + "," + agentTask, codexUA, true, "chat_completions", []string{"user", "assistant", "tool", "user"}, true, true},
		{"disabled", baseline + "," + agentTask, codexUA, false, "chat_completions", []string{"user"}, false, false},
		{"other_client", baseline + "," + agentTask, "unrelated-client/1.0", true, "chat_completions", []string{"user"}, false, false},
		{"ordinary_message", baseline, codexUA, true, "chat_completions", []string{"user"}, false, false},
		{"responses_passthrough", baseline + "," + agentTask, codexUA, true, "responses", nil, false, false},
	}
	for _, tc := range cases {
		for _, namespaced := range []bool{false, true} {
			for _, stream := range []bool{false, true} {
				t.Run(fmt.Sprintf("%s/namespaced=%t/stream=%t", tc.name, namespaced, stream), func(t *testing.T) {
					captured := make(chan []byte, 1)
					upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
						body, err := io.ReadAll(r.Body)
						if err != nil {
							t.Errorf("read upstream request: %v", err)
							w.WriteHeader(http.StatusBadRequest)
							return
						}
						captured <- body
						wantPath := "/v1/chat/completions"
						if tc.wireAPI == "responses" {
							wantPath = "/v1/responses"
						}
						if r.URL.Path != wantPath || r.Header.Get("Authorization") != "Bearer upstream-key" {
							t.Errorf("unexpected upstream path or auth: path=%s", r.URL.Path)
						}
						if tc.wireAPI == "responses" {
							if stream {
								w.Header().Set("Content-Type", "text/event-stream")
								_, _ = io.WriteString(w, "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"status\":\"completed\",\"output\":[]}}\n\n")
							} else {
								w.Header().Set("Content-Type", "application/json")
								_, _ = io.WriteString(w, `{"id":"resp_test","object":"response","status":"completed","output":[]}`)
							}
							return
						}
						if stream {
							w.Header().Set("Content-Type", "text/event-stream")
							_, _ = fmt.Fprintf(w, "data: {\"id\":\"chatcmpl_test\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":%q,\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n", model)
						} else {
							w.Header().Set("Content-Type", "application/json")
							_, _ = fmt.Fprintf(w, `{"id":"chatcmpl_test","object":"chat.completion","created":1,"model":%q,"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`, model)
						}
					}))
					defer upstream.Close()

					gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "upstream-key", UpstreamModel: model, UpstreamModels: []string{model}, WireAPI: tc.wireAPI}
					key := &apiKeySpec{ID: "test-key", Key: "client-key", Enabled: true, ProviderGateway: gateway}
					clientModel := model
					if namespaced {
						key.ProviderGateway = nil
						key.ModelRouting = &modelRoutingSpec{DefaultRoute: "oauth", FailurePolicy: "strict", Routes: []modelRouteSpec{{ID: "test-route", Namespace: "test", ProviderGateway: gateway}}}
						clientModel = "test/" + model
					}
					m := &manifest{APIKeys: []apiKeySpec{*key}, ModelIDs: []string{clientModel}, apiKeyByValue: map[string]*apiKeySpec{"client-key": key}}
					cfg := &config.Config{}
					cfg.Codex.OptimizeMultiAgentV2 = tc.enabled
					runtime := &fakeRuntime{}
					router := (&relayServer{runtime: runtime, cfg: cfg, manifest: m, policy: &requestPolicy{manifest: m}}).router()
					relay := httptest.NewServer(router)
					defer relay.Close()
					requestBody := fmt.Sprintf(`{"model":%q,"input":[%s],"stream":%t}`, clientModel, tc.input, stream)
					req, err := http.NewRequest(http.MethodPost, relay.URL+"/v1/responses", strings.NewReader(requestBody))
					if err != nil {
						t.Fatal(err)
					}
					req.Header.Set("Authorization", "Bearer client-key")
					req.Header.Set("Content-Type", "application/json")
					req.Header.Set("User-Agent", tc.userAgent)
					response, err := relay.Client().Do(req)
					if err != nil {
						t.Fatal(err)
					}
					defer response.Body.Close()
					responseBody, err := io.ReadAll(response.Body)
					if err != nil {
						t.Fatal(err)
					}
					if response.StatusCode != http.StatusOK {
						t.Fatalf("status=%d body=%s", response.StatusCode, responseBody)
					}
					var body []byte
					select {
					case body = <-captured:
					default:
						t.Fatal("request did not reach provider gateway upstream")
					}
					if runtime.executeCalls != 0 || runtime.streamCalls != 0 {
						t.Fatal("provider gateway must not enter the OAuth runtime")
					}
					if gjson.GetBytes(body, "model").String() != model || gjson.GetBytes(body, "stream").Bool() != stream {
						t.Fatalf("model or stream flag changed: %s", body)
					}
					if tc.wireAPI == "responses" {
						var before, after any
						if err := json.Unmarshal([]byte("["+tc.input+"]"), &before); err != nil {
							t.Fatal(err)
						}
						if err := json.Unmarshal([]byte(gjson.GetBytes(body, "input").Raw), &after); err != nil {
							t.Fatal(err)
						}
						if !reflect.DeepEqual(before, after) {
							t.Fatalf("native Responses input must remain untouched: %s", body)
						}
					} else {
						messages := gjson.GetBytes(body, "messages").Array()
						roles := make([]string, 0, len(messages))
						for _, message := range messages {
							roles = append(roles, message.Get("role").String())
						}
						if !reflect.DeepEqual(roles, tc.wantRoles) {
							t.Fatalf("upstream roles=%v want=%v body=%s", roles, tc.wantRoles, body)
						}
						if tc.wantTask {
							parts := messages[len(messages)-1].Get("content").Array()
							if len(parts) != 2 || parts[0].Get("text").String() != "Message Type: NEW_TASK\nPayload:\n" || parts[1].Get("text").String() != "Review only the assigned passage. 校验码：任务送达" {
								t.Fatalf("delegated task was changed or dropped: %s", body)
							}
						}
						if tc.wantToolID && (messages[1].Get("tool_calls.0.id").String() != "call_read" || messages[2].Get("tool_call_id").String() != "call_read" || messages[2].Get("content").String() != "assigned passage") {
							t.Fatalf("tool result pairing changed: %s", body)
						}
					}
					if stream && !strings.Contains(string(responseBody), "event: response.completed\n") {
						t.Fatalf("missing Responses stream completion: %s", responseBody)
					}
				})
			}
		}
	}
}
