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
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func xaiAgentContinuationEvents(turn int) [][]byte {
	output := make([]any, 0, 5)
	for _, name := range []string{"spawn_agent", "send_message", "followup_task"} {
		output = append(output, map[string]any{
			"type": "function_call", "name": "collaboration__" + name,
			"id": fmt.Sprintf("fc_%d_%s", turn, name), "call_id": fmt.Sprintf("call_%d_%s", turn, name),
			"arguments": `{"message":"plaintext task"}`, "status": "completed",
		})
	}
	output = append(output, map[string]any{
		"type": "function_call", "name": "collaboration__spawn_agent",
		"id": fmt.Sprintf("fc_%d_encrypted_args", turn), "call_id": fmt.Sprintf("call_%d_encrypted_args", turn),
		"arguments": `{"message":"opaque message"}`, "encrypted_function_args": []string{"existing-signature"}, "status": "completed",
	}, map[string]any{
		"type": "function_call", "name": "collaboration__send_message",
		"id": fmt.Sprintf("fc_%d_encrypted_content", turn), "call_id": fmt.Sprintf("call_%d_encrypted_content", turn),
		"arguments": `{"message":"opaque message"}`, "encrypted_content": "existing-content", "status": "completed",
	})
	events := []map[string]any{{"type": "response.created", "response": map[string]any{
		"id": fmt.Sprintf("resp_agent_%d", turn), "status": "in_progress",
	}}}
	for i, item := range output {
		events = append(events, map[string]any{"type": "response.output_item.done", "output_index": i, "item": item})
	}
	events = append(events, map[string]any{"type": "response.completed", "response": map[string]any{
		"id": fmt.Sprintf("resp_agent_%d", turn), "object": "response", "model": "grok-4.6", "status": "completed",
		"output": output, "usage": map[string]any{"input_tokens": 1, "output_tokens": 2, "total_tokens": 3},
	}})
	result := make([][]byte, 0, len(events))
	for i, event := range events {
		event["sequence_number"] = i
		raw, _ := json.Marshal(event)
		result = append(result, raw)
	}
	return result
}

func xaiAgentContinuationRequest(previous gjson.Result) []byte {
	tools := []any{map[string]any{"type": "namespace", "name": "collaboration", "tools": []any{
		map[string]any{"type": "function", "name": "spawn_agent", "parameters": map[string]any{"type": "object", "properties": map[string]any{"message": map[string]any{"type": "string"}}}},
		map[string]any{"type": "function", "name": "send_message", "parameters": map[string]any{"type": "object", "properties": map[string]any{"message": map[string]any{"type": "string"}}}},
		map[string]any{"type": "function", "name": "followup_task", "parameters": map[string]any{"type": "object", "properties": map[string]any{"message": map[string]any{"type": "string"}}}},
	}}}
	input := []any{map[string]any{
		"type": "agent_message", "id": "agent-original", "author": "/root", "recipient": "/root/worker",
		"content": []any{map[string]any{"type": "input_text", "text": "initial agent task"}, map[string]any{"type": "encrypted_content", "encrypted_content": "legacy-agent-bytes"}},
		"internal_chat_message_metadata_passthrough": map[string]any{"turn_id": "agent-turn"},
	}}
	body := map[string]any{"model": "grok-4.6", "tools": tools, "input": input}
	if previous.Exists() {
		for _, item := range previous.Get("output").Array() {
			input = append(input, json.RawMessage(item.Raw), map[string]any{
				"type": "function_call_output", "call_id": item.Get("call_id").String(), "output": "tool completed",
			})
		}
		input = append(input, map[string]any{
			"type": "agent_message", "author": "/root/worker", "recipient": "/root",
			"content": []any{map[string]any{"type": "input_text", "text": "followup agent reply"}},
		})
		body["input"] = input
		body["previous_response_id"] = previous.Get("id").String()
	}
	raw, _ := json.Marshal(body)
	return raw
}

func assertXAIAgentContinuationItem(t *testing.T, item gjson.Result) {
	t.Helper()
	if item.Get("type").String() != "function_call" || item.Get("namespace").String() != "collaboration" {
		t.Fatalf("collaboration identity was not restored: %s", item.Raw)
	}
	callID := item.Get("call_id").String()
	switch {
	case strings.HasSuffix(callID, "_encrypted_args"):
		if item.Get("encrypted_function_args").Raw != `["existing-signature"]` {
			t.Fatalf("explicit encrypted arguments were changed: %s", item.Raw)
		}
	case strings.HasSuffix(callID, "_encrypted_content"):
		if item.Get("encrypted_content").String() != "existing-content" || item.Get("encrypted_function_args").Exists() {
			t.Fatalf("explicit encrypted content was replaced with a plaintext marker: %s", item.Raw)
		}
	default:
		if item.Get("encrypted_function_args").Raw != "[]" {
			t.Fatalf("completed plaintext call lacks an explicit empty encryption marker: %s", item.Raw)
		}
		switch item.Get("name").String() {
		case "spawn_agent", "send_message", "followup_task":
		default:
			t.Fatalf("unrestored collaboration tool: %s", item.Raw)
		}
	}
}

func assertXAIAgentContinuationUpstream(t *testing.T, body []byte, followup bool) {
	t.Helper()
	agents, outputs := 0, 0
	foundLegacy, foundFollowup, foundMetadata := false, false, false
	for _, item := range gjson.GetBytes(body, "input").Array() {
		if item.Get("type").String() == "agent_message" {
			t.Fatalf("private agent_message reached xAI: %s", body)
		}
		if item.Get("type").String() == "function_call_output" {
			outputs++
			if !strings.HasPrefix(item.Get("call_id").String(), "call_1_") {
				t.Fatalf("tool result lost original call identity: %s", item.Raw)
			}
		}
		if item.Get("type").String() != "message" || item.Get("role").String() != "user" {
			continue
		}
		for _, key := range []string{"author", "recipient", "internal_chat_message_metadata_passthrough"} {
			if item.Get(key).Exists() {
				t.Fatalf("private routing metadata reached xAI message fields: %s", item.Raw)
			}
		}
		agents++
		for _, part := range item.Get("content").Array() {
			if part.Get("type").String() != "input_text" {
				t.Fatalf("nonportable agent content reached xAI: %s", part.Raw)
			}
			text := part.Get("text").String()
			foundLegacy = foundLegacy || text == "legacy-agent-bytes"
			foundFollowup = foundFollowup || text == "followup agent reply"
			foundMetadata = foundMetadata || strings.Contains(text, "Agent routing metadata:") && strings.Contains(text, "/root")
		}
	}
	if agents == 0 || !foundLegacy || !foundMetadata {
		t.Fatalf("agent projection lost content or routing information: %s", body)
	}
	if followup && (!foundFollowup || outputs != 5) {
		t.Fatalf("followup agent input or paired tool results were lost: %s", body)
	}
	for _, tool := range gjson.GetBytes(body, "tools").Array() {
		if tool.Get("type").String() != "function" || !strings.HasPrefix(tool.Get("name").String(), "collaboration__") {
			t.Fatalf("upstream collaboration declarations were not flattened: %s", body)
		}
	}
}

func xaiAgentContinuationStreamResult(t *testing.T, stream *cliproxyexecutor.StreamResult) gjson.Result {
	t.Helper()
	var completed gjson.Result
	done := 0
	for chunk := range stream.Chunks {
		if chunk.Err != nil {
			t.Fatal(chunk.Err)
		}
		var events [][]byte
		if gjson.ValidBytes(chunk.Payload) {
			events = append(events, chunk.Payload)
		} else {
			for _, line := range strings.Split(string(chunk.Payload), "\n") {
				if strings.HasPrefix(line, "data:") {
					events = append(events, []byte(strings.TrimSpace(strings.TrimPrefix(line, "data:"))))
				}
			}
		}
		for _, raw := range events {
			event := gjson.ParseBytes(raw)
			switch event.Get("type").String() {
			case "response.output_item.done":
				assertXAIAgentContinuationItem(t, event.Get("item"))
				done++
			case "response.completed":
				completed = event.Get("response")
			}
		}
	}
	if done != 5 || !completed.Exists() {
		t.Fatalf("missing completed tool lifecycle: done=%d response=%s", done, completed.Raw)
	}
	return completed
}

func TestXAIAgentContinuationWithoutOptimizationAcrossTransports(t *testing.T) {
	for _, transport := range []string{"http", "sse", "websocket"} {
		t.Run(transport, func(t *testing.T) {
			captured := make(chan []byte, 2)
			var turns atomic.Int32
			upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if websocket.IsWebSocketUpgrade(r) {
					conn, err := upgrader.Upgrade(w, r, nil)
					if err != nil {
						t.Error(err)
						return
					}
					defer func() { _ = conn.Close() }()
					for {
						_, body, errRead := conn.ReadMessage()
						if errRead != nil {
							return
						}
						captured <- body
						for _, event := range xaiAgentContinuationEvents(int(turns.Add(1))) {
							if errWrite := conn.WriteMessage(websocket.TextMessage, event); errWrite != nil {
								t.Error(errWrite)
								return
							}
						}
					}
				}
				body, err := io.ReadAll(r.Body)
				if err != nil {
					t.Error(err)
					http.Error(w, "read request", 500)
					return
				}
				captured <- body
				w.Header().Set("Content-Type", "text/event-stream")
				for _, event := range xaiAgentContinuationEvents(int(turns.Add(1))) {
					_, _ = fmt.Fprintf(w, "event: %s\ndata: %s\n\n", gjson.GetBytes(event, "type").String(), event)
				}
			}))
			defer server.Close()
			cfg := &config.Config{}
			cfg.Codex.OptimizeMultiAgentV2 = false
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, ResponseFormat: sdktranslator.FormatOpenAIResponse, Headers: http.Header{"User-Agent": []string{"fixture-client/1.0"}}}
			auth := &cliproxyauth.Auth{ID: "agent-continuation-" + transport, Provider: "xai", Attributes: map[string]string{"base_url": server.URL, "api_key": "fixture"}}
			ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel()
			httpExecutor := NewXAIExecutor(cfg)
			var wsExecutor *XAIWebsocketsExecutor
			if transport == "websocket" {
				wsExecutor = NewXAIWebsocketsExecutor(cfg)
				wsExecutor.store = &codexWebsocketSessionStore{sessions: make(map[string]*codexWebsocketSession)}
				wsExecutor.idStore = &xaiWebsocketIDStateStore{sessions: make(map[string]*xaiWebsocketIDState)}
				opts.Metadata = map[string]any{cliproxyexecutor.ExecutionSessionMetadataKey: "agent-continuation-session"}
				auth.Attributes["websockets"] = "true"
				ctx = cliproxyexecutor.WithDownstreamWebsocket(ctx)
				defer wsExecutor.CloseExecutionSession("agent-continuation-session")
			}
			var previous gjson.Result
			for turn := 0; turn < 2; turn++ {
				req := cliproxyexecutor.Request{Model: "grok-4.6", Payload: xaiAgentContinuationRequest(previous)}
				if transport == "http" {
					response, err := httpExecutor.Execute(ctx, auth, req, opts)
					if err != nil {
						t.Fatal(err)
					}
					previous = gjson.ParseBytes(response.Payload)
				} else {
					var stream *cliproxyexecutor.StreamResult
					var err error
					if wsExecutor != nil {
						stream, err = wsExecutor.ExecuteStream(ctx, auth, req, opts)
					} else {
						stream, err = httpExecutor.ExecuteStream(ctx, auth, req, opts)
					}
					if err != nil {
						t.Fatal(err)
					}
					previous = xaiAgentContinuationStreamResult(t, stream)
				}
				if len(previous.Get("output").Array()) != 5 {
					t.Fatalf("missing tools: %s", previous.Raw)
				}
				for _, item := range previous.Get("output").Array() {
					assertXAIAgentContinuationItem(t, item)
				}
				assertXAIAgentContinuationUpstream(t, <-captured, turn == 1)
			}
		})
	}
}
