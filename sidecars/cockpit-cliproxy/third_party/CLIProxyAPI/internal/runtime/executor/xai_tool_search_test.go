package executor

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

const xaiSearchTestRequest = `{"model":"grok-4.6","tools":[{"type":"tool_search","execution":"client","description":"Find task tools","parameters":{"type":"object","properties":{"goal":{"type":"string"}},"required":["goal"]}},{"type":"function","name":"__codex_tool_search"}],"input":[{"type":"tool_search_call","execution":"client","id":"tsc_old","call_id":"old_call","arguments":{"goal":"Find agents"}},{"type":"tool_search_output","execution":"client","call_id":"old_call","tools":[{"type":"namespace","name":"agents","tools":[{"type":"function","name":"spawn_agent","parameters":{"type":"object","properties":{"task":{"type":"string"}}}}]}]}]}`

func xaiSearchTestEvents(alias string) [][]byte {
	item := func(name, id, call, args string) map[string]any {
		return map[string]any{"type": "function_call", "name": name, "id": id, "call_id": call, "arguments": args}
	}
	search := item(alias, "fc_search", "call_search", `{"goal":"Find connectors"}`)
	agent := item("agents__spawn_agent", "fc_agent", "call_agent", `{"task":"Inspect"}`)
	events := []map[string]any{
		{"type": "response.output_item.added", "output_index": 0, "item": item(alias, "fc_search", "call_search", "")},
		{"type": "response.function_call_arguments.delta", "output_index": 0, "item_id": "fc_search", "delta": `{"goal":"Find connectors"}`},
		{"type": "response.function_call_arguments.done", "output_index": 0, "item_id": "fc_search", "arguments": `{"goal":"Find connectors"}`},
		{"type": "response.output_item.done", "output_index": 0, "item": search},
		{"type": "response.output_item.added", "output_index": 1, "item": agent},
		{"type": "response.output_item.done", "output_index": 1, "item": agent},
		{"type": "response.completed", "response": map[string]any{"id": "resp_search", "object": "response", "model": "grok-4.6", "status": "completed", "output": []any{search, agent}, "usage": map[string]any{"input_tokens": 1, "output_tokens": 2, "total_tokens": 3}}},
	}
	out := make([][]byte, 0, len(events))
	for i, event := range events {
		event["sequence_number"] = i
		raw, _ := json.Marshal(event)
		out = append(out, raw)
	}
	return out
}

func assertXAISearchUpstream(t *testing.T, body []byte) string {
	t.Helper()
	alias := gjson.GetBytes(body, "tools.0.name").String()
	if alias != "__codex_tool_search_1" || gjson.GetBytes(body, "tools.0.parameters.required.0").String() != "goal" {
		t.Errorf("search schema/alias lost: %s", body)
	}
	if gjson.GetBytes(body, "input.0.type").String() != "function_call" || gjson.GetBytes(body, "input.0.call_id").String() != "old_call" || gjson.GetBytes(body, "input.1.type").String() != "function_call_output" {
		t.Errorf("history not lowered: %s", body)
	}
	found := false
	for _, tool := range gjson.GetBytes(body, "tools").Array() {
		if tool.Get("type").String() == "tool_search" {
			t.Errorf("client search leaked upstream: %s", tool)
		}
		if tool.Get("name").String() == "agents__spawn_agent" {
			found = true
		}
	}
	if !found {
		t.Errorf("discovered agent not callable: %s", body)
	}
	return alias
}

func assertXAISearchResult(t *testing.T, response gjson.Result) {
	t.Helper()
	search := response.Get("output.0")
	if search.Get("type").String() != "tool_search_call" || search.Get("execution").String() != "client" || search.Get("id").String() != "tsc_search" || search.Get("call_id").String() != "call_search" || search.Get("arguments.goal").String() != "Find connectors" {
		t.Fatalf("invalid client search: %s", search)
	}
	agent := response.Get("output.1")
	if agent.Get("namespace").String() != "agents" || agent.Get("name").String() != "spawn_agent" || agent.Get("call_id").String() != "call_agent" {
		t.Fatalf("agent namespace lost: %s", agent)
	}
}

func assertXAISearchEvents(t *testing.T, events [][]byte) {
	t.Helper()
	added, done, completed := false, false, false
	for _, raw := range events {
		event := gjson.ParseBytes(raw)
		switch event.Get("type").String() {
		case "response.output_item.added", "response.output_item.done":
			if event.Get("item.call_id").String() != "call_search" {
				continue
			}
			if event.Get("item.type").String() != "tool_search_call" || event.Get("item.execution").String() != "client" || !event.Get("item.arguments").IsObject() {
				t.Fatalf("search lifecycle lost: %s", raw)
			}
			if event.Get("type").String() == "response.output_item.added" {
				added = true
			} else {
				done = true
			}
		case "response.function_call_arguments.delta", "response.function_call_arguments.done":
			if event.Get("item_id").String() == "fc_search" {
				t.Fatalf("proxy argument event leaked: %s", raw)
			}
		case "response.completed":
			assertXAISearchResult(t, event.Get("response"))
			completed = true
		}
	}
	if !added || !done || !completed {
		t.Fatalf("missing lifecycle: added=%v done=%v completed=%v", added, done, completed)
	}
}

func TestXAIClientToolSearchHTTPTransports(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		alias := assertXAISearchUpstream(t, body)
		w.Header().Set("Content-Type", "text/event-stream")
		for _, event := range xaiSearchTestEvents(alias) {
			fmt.Fprintf(w, "event: %s\ndata: %s\n\n", gjson.GetBytes(event, "type").String(), event)
		}
	}))
	defer server.Close()
	executor := NewXAIExecutor(&config.Config{})
	auth := &cliproxyauth.Auth{ID: "search-http", Provider: "xai", Attributes: map[string]string{"base_url": server.URL, "api_key": "test"}}
	opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, ResponseFormat: sdktranslator.FormatOpenAIResponse}
	req := cliproxyexecutor.Request{Model: "grok-4.6", Payload: []byte(xaiSearchTestRequest)}
	response, err := executor.Execute(context.Background(), auth, req, opts)
	if err != nil {
		t.Fatal(err)
	}
	assertXAISearchResult(t, gjson.ParseBytes(response.Payload))
	stream, err := executor.ExecuteStream(context.Background(), auth, req, opts)
	if err != nil {
		t.Fatal(err)
	}
	var events [][]byte
	for chunk := range stream.Chunks {
		if chunk.Err != nil {
			t.Fatal(chunk.Err)
		}
		for _, line := range strings.Split(string(chunk.Payload), "\n") {
			if strings.HasPrefix(line, "data:") {
				events = append(events, []byte(strings.TrimSpace(strings.TrimPrefix(line, "data:"))))
			}
		}
	}
	assertXAISearchEvents(t, events)
}

func TestXAIClientToolSearchWebsocketTransport(t *testing.T) {
	upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer func() { _ = conn.Close() }()
		_, body, err := conn.ReadMessage()
		if err != nil {
			t.Error(err)
			return
		}
		alias := assertXAISearchUpstream(t, body)
		for _, event := range xaiSearchTestEvents(alias) {
			if err := conn.WriteMessage(websocket.TextMessage, event); err != nil {
				t.Error(err)
				return
			}
		}
	}))
	defer server.Close()
	executor := NewXAIWebsocketsExecutor(&config.Config{})
	auth := &cliproxyauth.Auth{ID: "search-ws", Provider: "xai", Attributes: map[string]string{"base_url": server.URL, "api_key": "test", "websockets": "true"}}
	ctx, cancel := context.WithTimeout(cliproxyexecutor.WithDownstreamWebsocket(context.Background()), 5*time.Second)
	defer cancel()
	stream, err := executor.ExecuteStream(ctx, auth, cliproxyexecutor.Request{Model: "grok-4.6", Payload: []byte(xaiSearchTestRequest)}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, ResponseFormat: sdktranslator.FormatOpenAIResponse})
	if err != nil {
		t.Fatal(err)
	}
	var events [][]byte
	for chunk := range stream.Chunks {
		if chunk.Err != nil {
			t.Fatal(chunk.Err)
		}
		events = append(events, chunk.Payload)
	}
	assertXAISearchEvents(t, events)
}

func TestXAIClientToolSearchSurvivesToolLimit(t *testing.T) {
	tools := []any{map[string]any{"type": "tool_search", "execution": "client"}}
	for i := 0; i < xaiMaxTools+5; i++ {
		tools = append(tools, map[string]any{"type": "namespace", "name": fmt.Sprintf("ns%d", i), "tools": []any{map[string]any{"type": "function", "name": "run"}}})
	}
	body, _ := json.Marshal(map[string]any{"model": "grok-4.6", "tools": tools, "input": []any{}})
	prepared, err := NewXAIExecutor(&config.Config{}).prepareResponsesRequest(context.Background(), cliproxyexecutor.Request{Model: "grok-4.6", Payload: body}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse}, true)
	if err != nil {
		t.Fatal(err)
	}
	if prepared.clientToolSearch == nil || !xaiHasFunctionToolNamed(prepared.body, prepared.clientToolSearch.Alias) || gjson.GetBytes(prepared.body, "tools.#").Int() != xaiMaxTools {
		t.Fatalf("search removed by cap: %s", prepared.body)
	}
}

func TestXAIClientToolSearchWebsocketInheritsOnlyContinuation(t *testing.T) {
	upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	captured := make(chan []byte, 3)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer func() { _ = conn.Close() }()
		for turn := 1; turn <= 3; turn++ {
			_, body, err := conn.ReadMessage()
			if err != nil {
				t.Error(err)
				return
			}
			captured <- body
			var output []any
			if turn == 1 {
				output = []any{map[string]any{"id": "fc_search", "call_id": "call_search", "type": "function_call", "name": gjson.GetBytes(body, "tools.0.name").String(), "arguments": `{"goal":"Find tools"}`}}
			}
			event, _ := json.Marshal(map[string]any{"type": "response.completed", "response": map[string]any{"id": fmt.Sprintf("resp_turn%d", turn), "status": "completed", "output": output}})
			if err := conn.WriteMessage(websocket.TextMessage, event); err != nil {
				t.Error(err)
				return
			}
		}
	}))
	defer server.Close()
	executor := NewXAIWebsocketsExecutor(&config.Config{})
	executor.store = &codexWebsocketSessionStore{sessions: make(map[string]*codexWebsocketSession)}
	executor.idStore = &xaiWebsocketIDStateStore{sessions: make(map[string]*xaiWebsocketIDState)}
	defer executor.CloseExecutionSession("search-inheritance")
	auth := &cliproxyauth.Auth{ID: "search-inheritance-auth", Provider: "xai", Attributes: map[string]string{"base_url": server.URL, "api_key": "test", "websockets": "true"}}
	opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, ResponseFormat: sdktranslator.FormatOpenAIResponse, Metadata: map[string]any{cliproxyexecutor.ExecutionSessionMetadataKey: "search-inheritance"}}
	ctx, cancel := context.WithTimeout(cliproxyexecutor.WithDownstreamWebsocket(context.Background()), 5*time.Second)
	defer cancel()
	requests := []string{
		xaiSearchTestRequest,
		`{"type":"response.create","model":"grok-4.6","previous_response_id":"resp_turn1","input":[{"type":"tool_search_output","execution":"client","call_id":"call_search","tools":[{"type":"function","name":"newly_discovered","parameters":{"type":"object"}}]}]}`,
		`{"type":"response.create","model":"grok-4.6","input":[{"role":"user","content":"A new task"}]}`,
	}
	for index, body := range requests {
		result, err := executor.ExecuteStream(ctx, auth, cliproxyexecutor.Request{Model: "grok-4.6", Payload: []byte(body)}, opts)
		if err != nil {
			t.Fatal(err)
		}
		for chunk := range result.Chunks {
			if chunk.Err != nil {
				t.Fatal(chunk.Err)
			}
		}
		upstream := <-captured
		switch index {
		case 1:
			if gjson.GetBytes(upstream, "tools.0.parameters.required.0").String() != "goal" || gjson.GetBytes(upstream, "input.0.type").String() != "function_call_output" {
				t.Fatalf("continuation lost schema/history: %s", upstream)
			}
			found := false
			for _, tool := range gjson.GetBytes(upstream, "tools").Array() {
				if tool.Get("name").String() == "newly_discovered" {
					found = true
				}
			}
			if !found {
				t.Fatalf("followup discovery not callable: %s", upstream)
			}
		case 2:
			if gjson.GetBytes(upstream, "tools").Exists() {
				t.Fatalf("new task inherited old tools: %s", upstream)
			}
		}
	}
}

func TestXAIClientToolSearchPreservesNativeServerDeclaration(t *testing.T) {
	body := []byte(`{"model":"grok-4.6","tools":[{"type":"tool_search","execution":"server","description":"Native search"}],"input":[]}`)
	prepared, err := NewXAIExecutor(&config.Config{}).prepareResponsesRequest(context.Background(), cliproxyexecutor.Request{Model: "grok-4.6", Payload: body}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse}, true)
	if err != nil {
		t.Fatal(err)
	}
	if prepared.clientToolSearch != nil || gjson.GetBytes(prepared.body, "tools.0.type").String() != "tool_search" || gjson.GetBytes(prepared.body, "tools.0.execution").String() != "server" {
		t.Fatalf("native server search was changed: %s", prepared.body)
	}
}
