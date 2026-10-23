package executor

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestXAIApplyPatchSparseSnapshotsAndIntegrity(t *testing.T) {
	for _, transport := range []string{"http", "sse", "ws"} {
		for _, scenario := range []string{"sparse_done", "sparse_terminal", "conflicting_terminal", "malformed", "truncated"} {
			t.Run(transport+"/"+scenario, func(t *testing.T) {
				upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					var conn *websocket.Conn
					if transport == "ws" {
						var err error
						conn, err = upgrader.Upgrade(w, r, nil)
						if err != nil {
							t.Error(err)
							return
						}
						defer func() { _ = conn.Close() }()
						if _, _, err = conn.ReadMessage(); err != nil {
							return
						}
					} else {
						w.Header().Set("Content-Type", "text/event-stream")
					}
					events := []string{`{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_patch","call_id":"call_patch","name":"apply_patch","arguments":""}}`}
					if scenario != "truncated" {
						args := `{"input":"*** Begin Patch\n*** End Patch"}`
						if scenario == "malformed" {
							args = `{}`
						}
						output := `[]`
						if scenario == "sparse_terminal" {
							output = `[{"type":"function_call","id":"fc_patch","call_id":"call_patch","name":"apply_patch"}]`
						} else if scenario == "conflicting_terminal" {
							output = `[{"type":"function_call","id":"fc_patch","call_id":"call_patch","name":"apply_patch","arguments":"{\"input\":\"different patch\"}"}]`
						}
						events = append(events, fmt.Sprintf(`{"type":"response.function_call_arguments.done","item_id":"fc_patch","output_index":0,"arguments":%q}`, args), `{"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_patch","call_id":"call_patch","name":"apply_patch"}}`, `{"type":"response.completed","response":{"id":"resp_patch","status":"completed","output":`+output+`}}`)
					}
					for _, event := range events {
						if conn != nil {
							_ = conn.WriteMessage(websocket.TextMessage, []byte(event))
						} else {
							_, _ = fmt.Fprintf(w, "data: %s\n\n", event)
						}
					}
				}))
				defer server.Close()
				auth := &cliproxyauth.Auth{ID: "patch-" + transport, Provider: "xai", Attributes: map[string]string{"base_url": server.URL, "api_key": "test"}}
				req := cliproxyexecutor.Request{Model: "grok-4.6", Payload: []byte(`{"model":"grok-4.6","input":[],"tools":[{"type":"custom","name":"apply_patch"}]}`)}
				opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, ResponseFormat: sdktranslator.FormatOpenAIResponse}
				var result []byte
				var gotErr error
				if transport == "http" {
					response, err := NewXAIExecutor(&config.Config{}).Execute(t.Context(), auth, req, opts)
					result, gotErr = response.Payload, err
				} else {
					var stream *cliproxyexecutor.StreamResult
					if transport == "ws" {
						stream, gotErr = NewXAIWebsocketsExecutor(&config.Config{}).ExecuteStream(cliproxyexecutor.WithDownstreamWebsocket(t.Context()), auth, req, opts)
					} else {
						stream, gotErr = NewXAIExecutor(&config.Config{}).ExecuteStream(t.Context(), auth, req, opts)
					}
					if gotErr == nil {
						for chunk := range stream.Chunks {
							if chunk.Err != nil {
								gotErr = chunk.Err
							}
							for _, line := range strings.Split(string(chunk.Payload), "\n") {
								line = strings.TrimSpace(strings.TrimPrefix(line, "data:"))
								if gjson.Get(line, "type").String() == "response.output_item.done" {
									result = []byte(gjson.Get(line, "item").Raw)
								}
							}
						}
					}
				}
				if scenario != "sparse_done" && scenario != "sparse_terminal" {
					if gotErr == nil {
						t.Fatalf("%s succeeded with damaged patch: %s", scenario, result)
					}
					return
				}
				if gotErr != nil {
					t.Fatal(gotErr)
				}
				if transport == "http" {
					result = []byte(gjson.GetBytes(result, "output.0").Raw)
				}
				if gjson.GetBytes(result, "type").String() != "custom_tool_call" || gjson.GetBytes(result, "input").String() != "*** Begin Patch\n*** End Patch" {
					t.Fatalf("complete arguments were lost: %s", result)
				}
			})
		}
	}
}

func TestXAIApplyPatchBuffersAreBounded(t *testing.T) {
	r := newXAIApplyPatchRestorer(true)
	r.applyStream([]byte(`{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"p","name":"apply_patch"}}`))
	r.applyStream([]byte(fmt.Sprintf(`{"type":"response.function_call_arguments.delta","item_id":"p","delta":%q}`, strings.Repeat("x", 4*1024*1024+1))))
	if r.finish() == nil {
		t.Fatal("unbounded patch arguments accepted")
	}
}

func TestXAIApplyPatchRejectsChangedIdentity(t *testing.T) {
	for _, item := range []string{
		`{"type":"function_call","id":"p","call_id":"different","name":"apply_patch","arguments":"{\"input\":\"patch\"}"}`,
		`{"type":"function_call","id":"other","call_id":"c","name":"apply_patch","arguments":"{\"input\":\"patch\"}"}`,
		`{"type":"function_call","id":"p","call_id":"c","name":"ordinary","arguments":"{\"input\":\"patch\"}"}`,
		`{"type":"message","id":"p","call_id":"c","arguments":"{\"input\":\"patch\"}"}`,
	} {
		r := newXAIApplyPatchRestorer(true)
		r.applyStream([]byte(`{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"p","call_id":"c","name":"apply_patch"}}`))
		r.applyStream([]byte(`{"type":"response.output_item.done","output_index":0,"item":` + item + `}`))
		if r.finish() == nil {
			t.Fatalf("changed patch identity accepted: %s", item)
		}
	}
}
