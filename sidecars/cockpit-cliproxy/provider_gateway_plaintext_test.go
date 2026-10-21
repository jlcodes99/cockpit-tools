package main

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/tidwall/gjson"
)

func TestProviderGatewayResponsesPlaintextCollaboration(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, tool := range []string{"spawn_agent", "send_message", "followup_task"} {
		for _, stream := range []bool{false, true} {
			for _, tc := range []struct {
				name        string
				declaration string
				metadata    string
				userAgent   string
				disabled    bool
				flat        bool
				wantPlain   bool
			}{
				{name: "plaintext", declaration: `"type":"string"`, wantPlain: true},
				{name: "explicit-false", declaration: `"type":"string","encrypted":false`, wantPlain: true},
				{name: "prepared-plaintext", declaration: `"type":"string","encrypted":true`, wantPlain: true},
				{name: "encrypted", declaration: `"type":"string","encrypted":true`, flat: true},
				{name: "undeclared"},
				{name: "signed", declaration: `"type":"string"`, metadata: `,"signature":"keep"`},
				{name: "encrypted-result", declaration: `"type":"string"`, metadata: `,"encrypted_function_args":["message"]`},
				{name: "disabled", declaration: `"type":"string"`, disabled: true},
				{name: "unrelated-client", declaration: `"type":"string"`, userAgent: "curl/8.0"},
			} {
				t.Run(fmt.Sprintf("%s/stream=%t/%s", tool, stream, tc.name), func(t *testing.T) {
					upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
						raw, _ := io.ReadAll(r.Body)
						namespace := "collaboration"
						if gjson.GetBytes(raw, "tools.0.name").String() == "collaboration-optimize" {
							namespace = "collaboration-optimize"
						}
						item := fmt.Sprintf(`{"type":"function_call","id":"fc_fixture","call_id":"call_fixture","namespace":%q,"name":%q,"arguments":"{\"message\":\"task body\"}"%s}`, namespace, tool, tc.metadata)
						response := `{"id":"resp_fixture","object":"response","status":"completed","output":[` + item + `]}`
						if stream {
							w.Header().Set("Content-Type", "text/event-stream")
							fmt.Fprintf(w, "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":%s}\n\n", item)
							fmt.Fprintf(w, "data: {\"type\":\"response.completed\",\"response\":%s}\n\ndata: [DONE]\n\n", response)
							return
						}
						w.Header().Set("Content-Type", "application/json")
						fmt.Fprint(w, response)
					}))
					defer upstream.Close()

					gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "fixture-upstream", UpstreamModels: []string{"fixture-model"}, WireAPI: "responses"}
					spec := apiKeySpec{ID: "fixture", Key: "fixture-client", Enabled: true, ProviderGateway: gateway}
					m := &manifest{APIKeys: []apiKeySpec{spec}, ModelIDs: []string{"fixture-model"}, apiKeyByValue: map[string]*apiKeySpec{"fixture-client": &spec}}
					cfg := &config.Config{}
					cfg.Codex.OptimizeMultiAgentV2 = !tc.disabled
					router := (&relayServer{runtime: &fakeRuntime{}, cfg: cfg, manifest: m, policy: &requestPolicy{manifest: m}}).router()
					tools := "[]"
					if tc.declaration != "" {
						tools = fmt.Sprintf(`[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":%q,"parameters":{"type":"object","properties":{"message":{%s}}}}]}]`, tool, tc.declaration)
						if tc.flat {
							tools = fmt.Sprintf(`[{"type":"function","name":%q,"parameters":{"type":"object","properties":{"message":{%s}}}}]`, "collaboration."+tool, tc.declaration)
						}
					}
					payload := fmt.Sprintf(`{"model":"fixture-model","stream":%t,"input":"test","tools":%s}`, stream, tools)
					request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(payload))
					request.Header.Set("Authorization", "Bearer fixture-client")
					request.Header.Set("Content-Type", "application/json")
					ua := tc.userAgent
					if ua == "" {
						ua = "Codex Desktop/0.151.0"
					}
					request.Header.Set("User-Agent", ua)
					response := httptest.NewRecorder()
					router.ServeHTTP(response, request)
					if response.Code != http.StatusOK {
						t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
					}
					var items []gjson.Result
					if stream {
						if !strings.Contains(response.Body.String(), "event: response.output_item.done\n") || !strings.Contains(response.Body.String(), "data: [DONE]\n") {
							t.Fatal("SSE framing was not preserved")
						}
						for _, line := range strings.Split(response.Body.String(), "\n") {
							if !strings.HasPrefix(line, "data: ") {
								continue
							}
							event := gjson.Parse(strings.TrimPrefix(line, "data: "))
							if event.Get("item").Exists() {
								items = append(items, event.Get("item"))
							}
							items = append(items, event.Get("response.output").Array()...)
						}
						if len(items) != 2 {
							t.Fatalf("expected item and terminal response, got %d", len(items))
						}
					} else {
						items = gjson.Get(response.Body.String(), "output").Array()
						if len(items) != 1 {
							t.Fatalf("expected one function call, got %d", len(items))
						}
					}
					for _, item := range items {
						got := item.Get("encrypted_function_args")
						if tc.wantPlain && got.Raw != "[]" {
							t.Fatalf("missing plaintext marker: %s", item.Raw)
						}
						if !tc.wantPlain && tc.name != "encrypted-result" && got.Exists() {
							t.Fatalf("unexpected plaintext marker: %s", item.Raw)
						}
						if tc.name == "encrypted-result" && got.Raw != `["message"]` {
							t.Fatalf("upstream encryption metadata changed: %s", item.Raw)
						}
						if tc.name == "signed" && item.Get("signature").String() != "keep" {
							t.Fatal("upstream signature changed")
						}
						var arguments map[string]string
						if err := json.Unmarshal([]byte(item.Get("arguments").String()), &arguments); err != nil || arguments["message"] != "task body" {
							t.Fatalf("task arguments changed: %s", item.Raw)
						}
						if item.Get("namespace").String() != "collaboration" || item.Get("name").String() != tool || item.Get("call_id").String() != "call_fixture" {
							t.Fatalf("tool identity changed: %s", item.Raw)
						}
					}
				})
			}
		}
	}
}

// Exercise the HTTP task/result loop. The client fixture only constructs a
// plaintext agent_message when the returned call explicitly permits it.
// This is a protocol regression, not a live model or desktop-client test.
func TestProviderGatewayCollaborationTaskReceiptRoundTrip(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, parentStream := range []bool{false, true} {
		for _, childStream := range []bool{false, true} {
			t.Run(fmt.Sprintf("parent=%t/child=%t", parentStream, childStream), func(t *testing.T) {
				const task = "TASK:fixture-nonce-7c9a"
				const receipt = "RECEIPT:fixture-nonce-7c9a"
				upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					raw, _ := io.ReadAll(r.Body)
					var item string
					switch gjson.GetBytes(raw, "input.0.content.0.text").String() {
					case "START":
						item = fmt.Sprintf(`{"type":"function_call","id":"fc_spawn","call_id":"call_spawn","name":"collaboration-optimize__spawn_agent","arguments":%q}`, `{"message":"`+task+`"}`)
					case task:
						if gjson.GetBytes(raw, "input.0.type").String() != "message" {
							http.Error(w, "private task envelope was not projected", http.StatusBadRequest)
							return
						}
						item = fmt.Sprintf(`{"type":"function_call","id":"fc_receipt","call_id":"call_receipt","name":"collaboration::send_message","arguments":%q}`, `{"message":"`+receipt+`"}`)
					case receipt:
						if gjson.GetBytes(raw, "input.0.type").String() != "message" {
							http.Error(w, "private receipt envelope was not projected", http.StatusBadRequest)
							return
						}
						item = `{"type":"message","id":"msg_complete","role":"assistant","content":[{"type":"output_text","text":"RECEIPT_OK"}]}`
					default:
						http.Error(w, "task or receipt body missing", http.StatusBadRequest)
						return
					}
					response := `{"id":"resp_fixture","object":"response","status":"completed","output":[` + item + `]}`
					if gjson.GetBytes(raw, "stream").Bool() {
						w.Header().Set("Content-Type", "text/event-stream")
						fmt.Fprintf(w, "data: {\"type\":\"response.output_item.done\",\"item\":%s}\n\ndata: {\"type\":\"response.completed\",\"response\":%s}\n\ndata: [DONE]\n\n", item, response)
						return
					}
					w.Header().Set("Content-Type", "application/json")
					fmt.Fprint(w, response)
				}))
				defer upstream.Close()
				gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "fixture-upstream", UpstreamModels: []string{"fixture-model"}, WireAPI: "responses"}
				spec := apiKeySpec{ID: "fixture", Key: "fixture-client", Enabled: true, ProviderGateway: gateway}
				m := &manifest{APIKeys: []apiKeySpec{spec}, ModelIDs: []string{"fixture-model"}, apiKeyByValue: map[string]*apiKeySpec{"fixture-client": &spec}}
				cfg := &config.Config{}
				cfg.Codex.OptimizeMultiAgentV2 = true
				router := (&relayServer{runtime: &fakeRuntime{}, cfg: cfg, manifest: m, policy: &requestPolicy{manifest: m}}).router()
				call := func(message, tool string, stream, agentMessage bool) gjson.Result {
					t.Helper()
					input := fmt.Sprintf(`{"type":"message","role":"user","content":[{"type":"input_text","text":%q}]}`, message)
					if agentMessage {
						input = fmt.Sprintf(`{"type":"agent_message","author":"/root","recipient":"/root/child","content":[{"type":"input_text","text":%q}]}`, message)
					}
					tools := fmt.Sprintf(`[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":%q,"parameters":{"properties":{"message":{"type":"string","encrypted":true}}}}]}]`, tool)
					payload := fmt.Sprintf(`{"model":"fixture-model","stream":%t,"input":[%s],"tools":%s}`, stream, input, tools)
					request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(payload))
					request.Header.Set("Authorization", "Bearer fixture-client")
					request.Header.Set("Content-Type", "application/json")
					request.Header.Set("User-Agent", "Codex Desktop/0.151.0")
					recorder := httptest.NewRecorder()
					router.ServeHTTP(recorder, request)
					if recorder.Code != http.StatusOK {
						t.Fatalf("status=%d body=%s", recorder.Code, recorder.Body.String())
					}
					if !stream {
						return gjson.Get(recorder.Body.String(), "output.0")
					}
					for _, line := range strings.Split(recorder.Body.String(), "\n") {
						if strings.HasPrefix(line, "data: ") {
							event := gjson.Parse(strings.TrimPrefix(line, "data: "))
							if event.Get("type").String() == "response.completed" {
								return event.Get("response.output.0")
							}
						}
					}
					t.Fatal("no terminal output received")
					return gjson.Result{}
				}
				readPlaintext := func(item gjson.Result, tool string) string {
					t.Helper()
					if item.Get("namespace").String() != "collaboration" || item.Get("name").String() != tool || item.Get("encrypted_function_args").Raw != "[]" {
						t.Fatalf("client cannot treat collaboration arguments as plaintext: %s", item.Raw)
					}
					return gjson.Get(item.Get("arguments").String(), "message").String()
				}
				delegated := readPlaintext(call("START", "spawn_agent", parentStream, false), "spawn_agent")
				returned := readPlaintext(call(delegated, "send_message", childStream, true), "send_message")
				complete := call(returned, "send_message", parentStream, true)
				if complete.Get("content.0.text").String() != "RECEIPT_OK" {
					t.Fatalf("parent did not receive receipt: %s", complete.Raw)
				}
			})
		}
	}
}
