package executor

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// All execution paths derive compatibility from the selected endpoint, even
// when legacy per-model metadata contains false or no explicit catalog exists.
// The optional optimizer remains scoped to recognized official client identities.
func TestCodexMultiAgentCPAParityAcrossExecutionPaths(t *testing.T) {
	for _, mode := range []string{"http", "sse", "buffered-sse", "compact", "ws", "ws-stream"} {
		for _, compat := range []bool{false, true} {
			for _, optimize := range []bool{false, true} {
				for _, ua := range []string{"codex_exec/0.1", "unrelated-client/1"} {
					t.Run(fmt.Sprintf("%s/compat=%v/optimize=%v/%s", mode, compat, optimize, ua), func(t *testing.T) {
						seen := make(chan []byte, 1)
						upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
						server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
							var payload []byte
							var socket *websocket.Conn
							if websocket.IsWebSocketUpgrade(r) {
								var err error
								socket, err = upgrader.Upgrade(w, r, nil)
								if err != nil {
									t.Errorf("upgrade: %v", err)
									return
								}
								defer func() { _ = socket.Close() }()
								_, payload, err = socket.ReadMessage()
								if err != nil {
									t.Errorf("read: %v", err)
									return
								}
							} else {
								payload, _ = io.ReadAll(r.Body)
							}
							seen <- payload
							namespace := gjson.GetBytes(payload, "input.0.tools.0.name").String()
							response := fmt.Sprintf(`{"id":"parity-response","status":"completed","output":[{"type":"function_call","id":"parity-item","call_id":"parity-call","namespace":%q,"name":"spawn_agent","arguments":"{\"message\":\"literal task\"}"}]}`, namespace)
							event := `{"type":"response.completed","response":` + response + `}`
							if socket != nil {
								_ = socket.WriteMessage(websocket.TextMessage, []byte(event))
								return
							}
							if mode == "compact" {
								w.Header().Set("Content-Type", "application/json")
								_, _ = w.Write([]byte(response))
								return
							}
							w.Header().Set("Content-Type", "text/event-stream")
							_, _ = fmt.Fprintf(w, "data: %s\n\n", event)
						}))
						defer server.Close()
						cfg := &config.Config{Codex: config.CodexConfig{OptimizeMultiAgentV2: optimize, StreamBootstrapBuffering: mode == "buffered-sse", OrphanDelegationCompatibility: true}, CodexKey: []config.CodexKey{{APIKey: "synthetic-key", BaseURL: server.URL, Models: []config.CodexModel{{Name: "upstream-model", Alias: "visible-model", IsCompat: compat}}}}}
						auth := &cliproxyauth.Auth{ID: "synthetic-auth", Provider: "codex", Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic-key"}}
						payload, errPayload := sjson.SetRawBytes(codexSpawnAgentTestPayload(), "input.-1", []byte(`{"type":"function_call_output","namespace":"codex_app","name":"create_thread","call_id":"stale","output":"<codex_delegation>literal child result</codex_delegation>"}`))
						if errPayload != nil {
							t.Fatal(errPayload)
						}
						req := cliproxyexecutor.Request{Model: "visible-model", Payload: payload}
						opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Headers: http.Header{"User-Agent": {ua}, "X-Openai-Subagent": {"collab_spawn"}}}
						var output []byte
						var err error
						executor := NewCodexExecutor(cfg)
						ws := NewCodexWebsocketsExecutor(cfg)
						switch mode {
						case "sse", "buffered-sse", "ws-stream":
							var result *cliproxyexecutor.StreamResult
							if mode == "ws-stream" {
								result, err = ws.ExecuteStream(context.Background(), auth, req, opts)
							} else {
								result, err = executor.ExecuteStream(context.Background(), auth, req, opts)
							}
							if err == nil {
								for chunk := range result.Chunks {
									if chunk.Err != nil {
										err = chunk.Err
										break
									}
									output = append(output, chunk.Payload...)
								}
							}
						default:
							var response cliproxyexecutor.Response
							if mode == "compact" {
								opts.Alt = "responses/compact"
							}
							if mode == "ws" {
								response, err = ws.Execute(context.Background(), auth, req, opts)
							} else {
								response, err = executor.Execute(context.Background(), auth, req, opts)
							}
							output = response.Payload
						}
						if err != nil {
							t.Fatalf("execute: %v", err)
						}
						payload = <-seen
						message := gjson.GetBytes(payload, "input.1")
						if message.Get("type").String() != "message" || message.Get("role").String() != "user" || message.Get("author").Exists() || message.Get("recipient").Exists() || message.Get("internal_chat_message_metadata_passthrough").Exists() {
							t.Fatalf("API-key target was not adapted automatically: %s", payload)
						}
						if message.Get("content.1.type").String() != "input_text" || message.Get("content.1.text").String() != "delegated task" || message.Get("content.1.encrypted_content").Exists() {
							t.Fatalf("literal content conversion missing: %s", payload)
						}
						orphan := gjson.GetBytes(payload, "input.2")
						if orphan.Get("type").String() != "message" || orphan.Get("role").String() != "user" || orphan.Get("content.0.text").String() != "Tool output from codex_app__create_thread:\n<codex_delegation>literal child result</codex_delegation>" {
							t.Fatalf("executor orphan rewrite did not run independently: %s", payload)
						}
						assertCodexSpawnAgentClientNamespace(t, output)
					})
				}
			}
		}
	}
}

func TestCodexMultiAgentCompatibilityUsesAuthProtocolInsteadOfLegacyModelBindings(t *testing.T) {
	cfg := &config.Config{CodexKey: []config.CodexKey{{APIKey: "key", BaseURL: "https://synthetic.invalid", Models: []config.CodexModel{{Name: "actual", Alias: "visible", IsCompat: false}}}}}
	e := NewCodexExecutor(cfg)
	auth := &cliproxyauth.Auth{Provider: "codex", Attributes: map[string]string{"api_key": "key", "base_url": "https://synthetic.invalid"}}
	for _, model := range []string{"actual", "visible", "visible(high)", "unknown"} {
		base := strings.Split(model, "(")[0]
		if !e.resolveCodexModelIsCompat(auth, cliproxyexecutor.Request{Model: model}, base) {
			t.Fatalf("public target %s needs automatic compatibility", model)
		}
	}
	for _, value := range []bool{false, true} {
		req := cliproxyexecutor.Request{Model: "actual", Metadata: map[string]any{"cliproxy.resolved_api_key_model_info": &registry.ModelInfo{IsCompat: value}}}
		if !e.resolveCodexModelIsCompat(auth, req, "actual") {
			t.Fatalf("legacy resolved capability=%v disabled protocol adaptation", value)
		}
	}
	oauth := &cliproxyauth.Auth{Provider: "codex", Metadata: map[string]any{"access_token": "synthetic-access", "cockpit_model_compat": map[string]any{"actual": true}}}
	if e.resolveCodexModelIsCompat(oauth, cliproxyexecutor.Request{Model: "actual"}, "actual") {
		t.Fatal("legacy model binding changed the official Codex protocol")
	}
	cfg.CodexKey[0].Models = nil
	if !e.resolveCodexModelIsCompat(auth, cliproxyexecutor.Request{Model: "actual"}, "actual") || len(cfg.CodexKey[0].Models) != 0 {
		t.Fatal("automatic API-key compatibility must not require or narrow a model catalog")
	}
	if e.resolveCodexModelIsCompat(nil, cliproxyexecutor.Request{}, "") {
		t.Fatal("an unselected target has no compatibility policy")
	}
}
