package executor

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func xaiNamespaceStreamEvent(t *testing.T, event map[string]any) []byte {
	t.Helper()
	data, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func xaiNamespaceStreamFixture(t *testing.T, terminal string) [][]byte {
	t.Helper()
	var events [][]byte
	add := func(event map[string]any) {
		event["sequence_number"] = len(events)
		events = append(events, xaiNamespaceStreamEvent(t, event))
	}
	item := func(index int, arguments string) map[string]any {
		return map[string]any{"id": fmt.Sprintf("fc_%d", index), "call_id": fmt.Sprintf("call_%d", index), "type": "function_call", "name": "ns", "arguments": arguments}
	}
	args := []string{`{"name":"lookup","arguments":{"q":"test","id":9223372036854775807}}`, `{"arguments":"{\"text\":\"hello\\nworld\"}","name":"write"}`}
	for index := range args {
		add(map[string]any{"type": "response.output_item.added", "output_index": index, "item": item(index, "")})
	}
	// Interleave fragments, including inside names, JSON strings and escapes.
	for offset := 0; offset < len(args[0]) || offset < len(args[1]); offset += 3 {
		for index, arguments := range args {
			if offset >= len(arguments) {
				continue
			}
			end := min(offset+3, len(arguments))
			add(map[string]any{"type": "response.function_call_arguments.delta", "output_index": index, "item_id": fmt.Sprintf("fc_%d", index), "delta": arguments[offset:end]})
		}
	}
	for index, arguments := range args {
		if terminal == "arguments.done" {
			add(map[string]any{"type": "response.function_call_arguments.done", "output_index": index, "item_id": fmt.Sprintf("fc_%d", index), "arguments": arguments})
		}
		if terminal != "response.completed" {
			add(map[string]any{"type": "response.output_item.done", "output_index": index, "item": item(index, arguments)})
		}
	}
	add(map[string]any{"type": "response.completed", "response": map[string]any{"id": "resp_1", "object": "response", "model": "grok-4.6", "status": "completed", "output": []any{item(0, args[0]), item(1, args[1])}}})
	return events
}

func assertXAINamespaceStream(t *testing.T, events [][]byte) {
	t.Helper()
	names := map[string]string{"fc_0": "lookup", "fc_1": "write"}
	wantArgs := map[string]string{"fc_0": `{"q":"test","id":9223372036854775807}`, "fc_1": `{"text":"hello\nworld"}`}
	added := make(map[string]bool)
	arguments := make(map[string]string)
	lastSequence := int64(-1)
	for _, event := range events {
		if seq := gjson.GetBytes(event, "sequence_number"); seq.Exists() {
			if seq.Int() <= lastSequence {
				t.Fatalf("non-monotonic sequence: %s after %d", event, lastSequence)
			}
			lastSequence = seq.Int()
		}
		itemID := gjson.GetBytes(event, "item_id").String()
		switch gjson.GetBytes(event, "type").String() {
		case "response.output_item.added", "response.output_item.done":
			item := gjson.GetBytes(event, "item")
			id := item.Get("id").String()
			if item.Get("name").String() != names[id] || item.Get("namespace").String() != "ns" || item.Get("call_id").String() != strings.Replace(id, "fc_", "call_", 1) {
				t.Fatalf("wrong tool identity: %s", event)
			}
			if gjson.GetBytes(event, "type").String() == "response.output_item.added" {
				if added[id] || item.Get("arguments").String() != "" {
					t.Fatalf("duplicated added or initial arguments: %s", event)
				}
				added[id] = true
			} else if item.Get("arguments").String() != arguments[id] {
				t.Fatalf("item.done disagrees with deltas: %s; deltas=%s", event, arguments[id])
			}
		case "response.function_call_arguments.delta":
			if !added[itemID] {
				t.Fatalf("delta before added: %s", event)
			}
			arguments[itemID] += gjson.GetBytes(event, "delta").String()
		case "response.function_call_arguments.done":
			if gjson.GetBytes(event, "arguments").String() != arguments[itemID] {
				t.Fatalf("arguments.done disagrees with deltas: %s; deltas=%s", event, arguments[itemID])
			}
		case "response.completed":
			for _, item := range gjson.GetBytes(event, "response.output").Array() {
				id := item.Get("id").String()
				if item.Get("name").String() != names[id] || item.Get("namespace").String() != "ns" || item.Get("arguments").String() != arguments[id] {
					t.Fatalf("completed disagrees with incremental result: %s; deltas=%v", event, arguments)
				}
			}
		}
	}
	for id, want := range wantArgs {
		if !added[id] || arguments[id] != want {
			t.Fatalf("incremental result for %s = %q, want %q", id, arguments[id], want)
		}
	}
}

func TestXAINamespaceStreamDispatcherInterleavedCalls(t *testing.T) {
	for _, terminal := range []string{"arguments.done", "output_item.done", "response.completed"} {
		t.Run(terminal, func(t *testing.T) {
			restorer := newXAINamespaceRestorer(map[string]xaiNamespaceToolRef{"ns": {namespace: "ns", isDispatcher: true}})
			var events [][]byte
			for _, event := range xaiNamespaceStreamFixture(t, terminal) {
				events = append(events, restorer.restoreStream(event)...)
			}
			assertXAINamespaceStream(t, events)
			if len(restorer.pendingCalls) != 0 {
				t.Fatal("completed calls retained buffered state")
			}
		})
	}
}

func TestXAINamespaceStreamRestoresFlatIdentityImmediately(t *testing.T) {
	restorer := newXAINamespaceRestorer(map[string]xaiNamespaceToolRef{"ns__lookup": {namespace: "ns", name: "lookup"}})
	for _, eventType := range []string{"response.output_item.added", "response.output_item.done"} {
		event := xaiNamespaceStreamEvent(t, map[string]any{"type": eventType, "item": map[string]any{"id": "fc", "type": "function_call", "name": "ns__lookup", "arguments": "{}"}})
		got := restorer.restoreStream(event)
		if len(got) != 1 || gjson.GetBytes(got[0], "item.name").String() != "lookup" || gjson.GetBytes(got[0], "item.namespace").String() != "ns" {
			t.Fatalf("wrong flat namespace event: %s", got)
		}
	}
}

func TestXAINamespaceStreamPreservesOrdinaryEvents(t *testing.T) {
	restorer := newXAINamespaceRestorer(map[string]xaiNamespaceToolRef{"ns": {namespace: "ns", isDispatcher: true}})
	events := []string{
		`{ "type":"response.output_item.added", "item":{"id":"fc","type":"function_call","name":"ordinary","arguments":""}}`,
		`{"type":"response.function_call_arguments.delta","item_id":"fc","delta":"{\"name\":\"user\"}"}`,
		`{"type":"response.function_call_arguments.done","item_id":"fc","arguments":"{\"name\":\"user\"}"}`,
		`{"type":"response.output_item.done","item":{"id":"fc","type":"function_call","name":"ordinary","arguments":"{}"}}`,
		`{"type":"response.completed","response":{"output":[]}}`,
		`[DONE]`,
	}
	for _, event := range events {
		got := restorer.restoreStream([]byte(event))
		if len(got) != 1 || !bytes.Equal(got[0], []byte(event)) {
			t.Fatalf("ordinary event mutated: %s => %s", event, got)
		}
	}
}

func TestXAINamespaceStreamDispatcherDoesNotBlockOtherEvents(t *testing.T) {
	restorer := newXAINamespaceRestorer(map[string]xaiNamespaceToolRef{"ns": {namespace: "ns", isDispatcher: true}})
	message := []byte(`{"type":"response.output_item.added","output_index":0,"item":{"id":"msg","type":"message","content":[]}}`)
	if got := restorer.restoreStream(message); len(got) != 1 {
		t.Fatalf("initial message was not published: %s", got)
	}
	added := []byte(`{"type":"response.output_item.added","output_index":1,"item":{"id":"fc","call_id":"call","type":"function_call","name":"ns","arguments":""}}`)
	if got := restorer.restoreStream(added); len(got) != 0 {
		t.Fatalf("dispatcher identity emitted before child is known: %s", got)
	}
	textEvent := []byte(`{"type":"response.output_text.delta","output_index":0,"delta":"Still working"}`)
	if got := restorer.restoreStream(textEvent); len(got) != 1 || !bytes.Equal(got[0], textEvent) {
		t.Fatalf("unrelated event blocked or changed: %s", got)
	}
	// No argument delta: the final event alone must still supply an incremental
	// client with both the real tool identity and the complete child arguments.
	done := []byte(`{"type":"response.function_call_arguments.done","output_index":1,"item_id":"fc","arguments":"{\"name\":\"lookup\",\"q\":\"test\"}"}`)
	got := restorer.restoreStream(done)
	if len(got) != 3 || gjson.GetBytes(got[0], "item.name").String() != "lookup" || gjson.GetBytes(got[0], "item.namespace").String() != "ns" {
		t.Fatalf("missing restored added: %s", got)
	}
	if gjson.GetBytes(got[1], "item_id").String() != "fc" || gjson.GetBytes(got[1], "delta").String() != `{"q":"test"}` || gjson.GetBytes(got[2], "arguments").String() != `{"q":"test"}` {
		t.Fatalf("missing or inconsistent arguments: %s", got)
	}
}

func TestXAINamespaceStreamPreservesSDKOutputArrayOrder(t *testing.T) {
	restorer := newXAINamespaceRestorer(map[string]xaiNamespaceToolRef{"ns": {namespace: "ns", isDispatcher: true}})
	fixture := xaiNamespaceStreamFixture(t, "arguments.done")
	var input [][]byte
	var done [2][][]byte
	for _, event := range fixture[:len(fixture)-1] {
		switch gjson.GetBytes(event, "type").String() {
		case "response.function_call_arguments.done", "response.output_item.done":
			index := gjson.GetBytes(event, "output_index").Int()
			done[index] = append(done[index], event)
		default:
			input = append(input, event)
		}
	}
	input = append(input,
		[]byte(`{"type":"response.output_item.added","output_index":2,"item":{"id":"msg","type":"message","content":[]}}`),
		[]byte(`{"type":"response.content_part.added","output_index":2,"content_index":0,"part":{"type":"output_text","text":""}}`),
		[]byte(`{"type":"response.output_text.delta","output_index":2,"content_index":0,"delta":"hello"}`),
	)
	// The second dispatcher finishes first. Neither it nor the later message
	// may be appended ahead of the still-unresolved output at index zero.
	input = append(input, done[1]...)
	input = append(input, done[0]...)
	input = append(input, fixture[len(fixture)-1])
	type clientItem struct {
		id, name, arguments string
		content             []string
	}
	var output []clientItem
	for _, upstream := range input {
		for _, event := range restorer.restoreStream(upstream) {
			eventType := gjson.GetBytes(event, "type").String()
			index := int(gjson.GetBytes(event, "output_index").Int())
			if eventType == "response.output_item.added" {
				if index != len(output) {
					t.Fatalf("SDK append would assign output index %d instead of %d: %s", len(output), index, event)
				}
				item := gjson.GetBytes(event, "item")
				output = append(output, clientItem{id: item.Get("id").String(), name: item.Get("name").String(), arguments: item.Get("arguments").String()})
				continue
			}
			if !gjson.GetBytes(event, "output_index").Exists() {
				continue
			}
			if index >= len(output) {
				t.Fatalf("SDK array access before added at index %d: %s", index, event)
			}
			item := &output[index]
			switch eventType {
			case "response.function_call_arguments.delta":
				if item.id != gjson.GetBytes(event, "item_id").String() {
					t.Fatalf("SDK delta targeted wrong output: %s", event)
				}
				item.arguments += gjson.GetBytes(event, "delta").String()
			case "response.function_call_arguments.done":
				if item.arguments != gjson.GetBytes(event, "arguments").String() {
					t.Fatalf("SDK arguments mismatch: %s", event)
				}
			case "response.content_part.added":
				item.content = append(item.content, gjson.GetBytes(event, "part.text").String())
			case "response.output_text.delta":
				contentIndex := int(gjson.GetBytes(event, "content_index").Int())
				if contentIndex >= len(item.content) {
					t.Fatalf("SDK text delta before content part: %s", event)
				}
				item.content[contentIndex] += gjson.GetBytes(event, "delta").String()
			}
		}
	}
	if len(output) != 3 || output[0].name != "lookup" || output[1].name != "write" || output[2].id != "msg" || len(output[2].content) != 1 || output[2].content[0] != "hello" {
		t.Fatalf("SDK incremental output mismatch: %+v", output)
	}
}

func TestXAIExecutorNamespaceDispatcherStreamProtocol(t *testing.T) {
	fixture := xaiNamespaceStreamFixture(t, "arguments.done")
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		for _, event := range fixture {
			_, _ = fmt.Fprintf(w, "event: %s\ndata: %s\n\n", gjson.GetBytes(event, "type").String(), event)
		}
	}))
	defer server.Close()
	var tools []map[string]any
	for index := 0; index < 201; index++ {
		name := fmt.Sprintf("tool_%d", index)
		if index == 0 {
			name = "lookup"
		} else if index == 1 {
			name = "write"
		}
		tools = append(tools, map[string]any{"type": "function", "name": name, "parameters": map[string]any{"type": "object"}})
	}
	payload := xaiNamespaceStreamEvent(t, map[string]any{"model": "grok-4.6", "input": "call tools", "tools": []any{map[string]any{"type": "namespace", "name": "ns", "tools": tools}}})
	exec := NewXAIExecutor(&config.Config{})
	result, err := exec.ExecuteStream(context.Background(), &cliproxyauth.Auth{Provider: "xai", Attributes: map[string]string{"base_url": server.URL}, Metadata: map[string]any{"access_token": "test-token"}}, cliproxyexecutor.Request{Model: "grok-4.6", Payload: payload}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, ResponseFormat: sdktranslator.FormatOpenAIResponse, Stream: true})
	if err != nil {
		t.Fatal(err)
	}
	var stream bytes.Buffer
	for chunk := range result.Chunks {
		if chunk.Err != nil {
			t.Fatal(chunk.Err)
		}
		stream.Write(chunk.Payload)
		stream.WriteByte('\n')
	}
	var events [][]byte
	var eventName string
	for _, line := range strings.Split(stream.String(), "\n") {
		if strings.HasPrefix(line, "event: ") {
			eventName = strings.TrimPrefix(line, "event: ")
		} else if strings.HasPrefix(line, "data: ") {
			event := []byte(strings.TrimPrefix(line, "data: "))
			if name := gjson.GetBytes(event, "type").String(); name != eventName {
				t.Fatalf("SSE event name %q differs from data %q: %s", eventName, name, stream.String())
			}
			events = append(events, event)
		}
	}
	assertXAINamespaceStream(t, events)
}
