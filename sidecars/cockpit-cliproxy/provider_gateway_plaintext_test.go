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
