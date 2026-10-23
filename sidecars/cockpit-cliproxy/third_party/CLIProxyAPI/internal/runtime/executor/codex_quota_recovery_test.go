package executor

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
)

const sharingQuotaEvent = `{"type":"response.failed","response":{"error":{"type":"invalid_request_error","code":"subscription_sharing_usage_limit_exceeded","message":"Synthetic sharing quota exhausted"}},"resets_in_seconds":120}`

// These accounts deliberately have no official authorization bindings. Both
// native OAuth and API-key routes must use the same safe pre-output recovery.
func TestCodexQuotaRecoveryWithoutOfficialBinding(t *testing.T) {
	for _, path := range []struct {
		transport string
		stream    bool
	}{{"http", true}, {"websocket", true}, {"http", false}, {"websocket", false}} {
		transport := path.transport
		mode := "buffered"
		if path.stream {
			mode = "stream"
		}
		for _, authKind := range []string{"oauth", "api-key"} {
			for _, tc := range []struct {
				name, metadata, terminal string
				wantRotate               bool
				noHandshake              bool
			}{
				{name: "delayed-sharing", terminal: sharingQuotaEvent, wantRotate: true},
				{name: "response-metadata", metadata: `{"type":"response.metadata","metadata":{"model":"synthetic"}}`, terminal: sharingQuotaEvent, wantRotate: true},
				{name: "codex-metadata", metadata: `{"type":"codex.response.metadata","metadata":{}}`, terminal: sharingQuotaEvent, wantRotate: true},
				{name: "usage-code", metadata: `{"type":"response.metadata"}`, terminal: `{"type":"error","error":{"code":"usage_limit_reached","resets_in_seconds":120}}`, wantRotate: true},
				{name: "metadata-text", metadata: `{"type":"response.metadata","metadata":{"text":"already generated"}}`, terminal: sharingQuotaEvent},
				{name: "metadata-tool", metadata: `{"type":"response.metadata","item":{"type":"function_call","name":"write_file","arguments":"{}"}}`, terminal: sharingQuotaEvent},
				{name: "metadata-reasoning", metadata: `{"type":"codex.response.metadata","response":{"reasoning":{"summary":[{"text":"already reasoned"}]}}}`, terminal: sharingQuotaEvent},
				{name: "metadata-usage", metadata: `{"type":"response.metadata","usage":{"output_tokens":1}}`, terminal: sharingQuotaEvent},
				{name: "output-before-quota", metadata: codexOutputAddedEvent, terminal: sharingQuotaEvent},
				{name: "reasoning-before-quota", metadata: `{"type":"response.reasoning_summary_text.delta","delta":"reasoned"}`, terminal: sharingQuotaEvent},
				{name: "tool-before-quota", metadata: `{"type":"response.function_call_arguments.delta","delta":"{}","item_id":"fc_new"}`, terminal: sharingQuotaEvent},
				{name: "terminal-output", terminal: `{"type":"response.failed","response":{"output":[{"type":"function_call","name":"write_file","arguments":"{}"}],"error":{"code":"subscription_sharing_usage_limit_exceeded"}}}`, noHandshake: true},
				{name: "terminal-ws-status-output", terminal: `{"type":"error","status":429,"response":{"output":[{"type":"function_call","name":"write_file","arguments":"{}"}]},"error":{"code":"subscription_sharing_usage_limit_exceeded"}}`, noHandshake: true},
				{name: "terminal-usage", terminal: `{"type":"response.failed","response":{"usage":{"output_tokens":1},"error":{"code":"subscription_sharing_usage_limit_exceeded"}}}`, noHandshake: true},
			} {
				t.Run(transport+"/"+mode+"/"+authKind+"/"+tc.name, func(t *testing.T) {
					requestBody := codexQuotaLongHistory(t)
					wantInput := gjson.GetBytes(requestBody, "input").Raw
					var callsA, callsB atomic.Int32
					upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
					upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
						first := r.Header.Get("Authorization") == "Bearer a-key"
						if first {
							callsA.Add(1)
						} else {
							callsB.Add(1)
						}
						var request []byte
						var conn *websocket.Conn
						if transport == "websocket" {
							var errUpgrade error
							conn, errUpgrade = upgrader.Upgrade(w, r, nil)
							if errUpgrade != nil {
								t.Errorf("upgrade: %v", errUpgrade)
								return
							}
							defer func() { _ = conn.Close() }()
							_, request, _ = conn.ReadMessage()
						} else {
							request, _ = io.ReadAll(r.Body)
							w.Header().Set("Content-Type", "text/event-stream")
						}
						if gjson.GetBytes(request, "input").Raw != wantInput {
							t.Error("recovery lost or changed long conversation/tool call pairs")
						}
						writeEvent := func(event string) {
							if conn != nil {
								_ = conn.WriteMessage(websocket.TextMessage, []byte(event))
							} else {
								_, _ = fmt.Fprintf(w, "data: %s\n\n", event)
								w.(http.Flusher).Flush()
							}
						}
						if !first {
							writeEvent(codexCreatedEvent)
							writeEvent(codexOutputAddedEvent)
							writeEvent(codexCompletedEventBody)
							return
						}
						if !tc.noHandshake {
							// Real response envelopes echo request configuration, including
							// schema properties named like output fields.
							writeEvent(`{"type":"response.created","response":{"output":[],"usage":null,"text":{"format":{"type":"text"},"verbosity":"medium"},"reasoning":{"effort":"low","summary":null},"tools":[{"type":"function","parameters":{"properties":{"content":{"type":"string"},"arguments":{"type":"string"}}}}]}}`)
							writeEvent(codexInProgressEvent)
						}
						if tc.metadata != "" {
							writeEvent(tc.metadata)
						}
						// Exceeds the old Rust 250ms boundary and proves Go has no
						// implicit elapsed-time release before a quota decision.
						time.Sleep(400 * time.Millisecond)
						writeEvent(tc.terminal)
					}))
					defer upstream.Close()
					cfg := codexBufferingConfig(true)
					manager := coreauth.NewManager(nil, nil, nil)
					manager.SetConfig(cfg)
					manager.SetRetryConfig(0, 0, 2)
					if transport == "websocket" {
						manager.RegisterExecutor(NewCodexWebsocketsExecutor(cfg))
					} else {
						manager.RegisterExecutor(NewCodexExecutor(cfg))
					}
					var firstID string
					for i, name := range []string{"a", "b"} {
						id := "quota-" + transport + "-" + mode + "-" + authKind + "-" + tc.name + "-" + name
						if i == 0 {
							firstID = id
						}
						registry.GetGlobalRegistry().RegisterClient(id, "codex", []*registry.ModelInfo{{ID: "gpt-5.6-terra"}})
						defer registry.GetGlobalRegistry().UnregisterClient(id)
						auth := &coreauth.Auth{ID: id, Provider: "codex", Status: coreauth.StatusActive,
							Attributes: map[string]string{"base_url": upstream.URL, "priority": fmt.Sprint(100 - i)}}
						if authKind == "oauth" {
							auth.Metadata = map[string]any{"type": "codex", "access_token": name + "-key"}
						} else {
							auth.Attributes["api_key"] = name + "-key"
						}
						if _, errRegister := manager.Register(context.Background(), auth); errRegister != nil {
							t.Fatal(errRegister)
						}
					}
					req, opts := codexTestRequest()
					req.Payload = requestBody
					var err, streamErr error
					var output string
					if path.stream {
						result, errStream := manager.ExecuteStream(context.Background(), []string{"codex"}, req, opts)
						err = errStream
						if result != nil {
							output, streamErr = drainChunks(result)
						}
					} else {
						result, errExecute := manager.Execute(context.Background(), []string{"codex"}, req, opts)
						err = errExecute
						output = string(result.Payload)
					}
					if tc.wantRotate {
						if err != nil || streamErr != nil || !(strings.Contains(output, "response.completed") || gjson.Get(output, "id").String() == "resp_1") || callsA.Load() != 1 || callsB.Load() != 1 {
							t.Fatalf("quota recovery failed: a=%d b=%d err=%v stream=%v output=%s", callsA.Load(), callsB.Load(), err, streamErr, output)
						}
					} else if callsA.Load() != 1 || callsB.Load() != 0 {
						t.Fatalf("executed request replayed: a=%d b=%d err=%v stream=%v", callsA.Load(), callsB.Load(), err, streamErr)
					}
					failed, _ := manager.GetByID(firstID)
					if !failed.Quota.Exceeded || !failed.NextRetryAfter.After(time.Now()) {
						t.Fatalf("real quota rejection did not cool credential: %+v", failed)
					}
					if tc.wantRotate && time.Until(failed.NextRetryAfter) < 115*time.Second {
						t.Fatalf("reset hint lost: next retry %v", failed.NextRetryAfter)
					}
				})
			}
		}
	}
}

func codexQuotaLongHistory(t *testing.T) []byte {
	t.Helper()
	input := make([]any, 0, 181)
	for i := 0; i < 80; i++ {
		input = append(input, map[string]any{"role": "user", "content": fmt.Sprintf("turn %d: inspect file", i)}, map[string]any{"role": "assistant", "content": fmt.Sprintf("turn %d: inspection complete", i)})
	}
	for i := 0; i < 10; i++ {
		id := fmt.Sprintf("call-%d", i)
		input = append(input, map[string]any{"type": "function_call", "call_id": id, "name": "read_file", "arguments": `{"path":"fixture.txt"}`}, map[string]any{"type": "function_call_output", "call_id": id, "output": "synthetic historical file content"})
	}
	input = append(input, map[string]any{"role": "user", "content": "Continue from this history without executing any tools"})
	body, err := json.Marshal(map[string]any{"model": "gpt-5.6-terra", "input": input, "tools": []any{map[string]any{"type": "function", "name": "read_file", "parameters": map[string]any{"type": "object", "properties": map[string]any{"path": map[string]any{"type": "string"}}}}}, "tool_choice": "none"})
	if err != nil {
		t.Fatal(err)
	}
	return body
}
