package executor

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/tidwall/gjson"
)

func TestNormalizeCodexInstructionsUsesModelBaseInstructions(t *testing.T) {
	for _, body := range [][]byte{
		[]byte(`{"model":"gpt-5.6-sol"}`),
		[]byte(`{"instructions":null}`),
		[]byte(`{"instructions":"  \n\t"}`),
	} {
		got := normalizeCodexInstructions(body, "gpt-5.6-sol")
		if instructions := gjson.GetBytes(got, "instructions").String(); len(instructions) < 100 {
			t.Fatalf("expected non-empty model base instructions, got %q from %s", instructions, body)
		}
	}

	body := []byte(`{"instructions":"keep me"}`)
	if got := gjson.GetBytes(normalizeCodexInstructions(body, "gpt-5.6-sol"), "instructions").String(); got != "keep me" {
		t.Fatalf("explicit instructions = %q, want keep me", got)
	}
}

func TestNormalizeCodexCallIDsRepairsMissingReplayIDs(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-flash","input":[
		{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"pwd\"}"},
		{"type":"function_call_output","output":"/workspace"},
		{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch"},
		{"type":"custom_tool_call_output","output":"Done!"},
		{"type":"function_call","call_id":"call_existing","name":"lookup","arguments":"{}"},
		{"type":"function_call_output","call_id":"call_existing","output":"ok"}
	]}`)

	got := normalizeCodexCallIDs(body)
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
	if gotID := gjson.GetBytes(got, "input.5.call_id").String(); gotID != "call_existing" {
		t.Fatalf("existing output call_id changed to %q", gotID)
	}
}

func TestNormalizeCodexCallIDsDropsAnonymousOrphanOutputs(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-flash","input":[
		{"type":"message","role":"user","content":"continue"},
		{"type":"function_call_output","output":"orphan result"},
		{"type":"function_call","name":"exec_command","arguments":"{}"},
		{"type":"function_call_output","output":"paired result"},
		{"type":"function_call_output","name":"heartbeat","output":"keep standalone"}
	]}`)

	got := normalizeCodexCallIDs(body)
	if strings.Contains(string(got), "call_missing_output") {
		t.Fatalf("anonymous orphan output received synthetic call_id: %s", got)
	}
	input := gjson.GetBytes(got, "input").Array()
	if len(input) != 4 {
		t.Fatalf("input len = %d, want 4: %s", len(input), got)
	}
	if input[1].Get("type").String() != "function_call" {
		t.Fatalf("orphan output was not removed before paired call: %s", got)
	}
	callID := input[1].Get("call_id").String()
	if callID == "" || input[2].Get("call_id").String() != callID {
		t.Fatalf("paired call/output IDs were not repaired together: %s", got)
	}
	if input[3].Get("name").String() != "heartbeat" || input[3].Get("call_id").Exists() {
		t.Fatalf("standalone named function_call_output was not preserved: %s", got)
	}
}

func TestNormalizeCodexCallIDsPreservesExistingReplayItems(t *testing.T) {
	body := []byte(`{"model":"gpt-5.6-sol","input":[
		{"type":"message","role":"user","content":"continue"},
		{"type":"function_call","call_id":"old_unanswered","name":"exec_command","arguments":"{}"},
		{"type":"function_call","call_id":"old_answered","name":"lookup","arguments":"{}"},
		{"type":"function_call_output","call_id":"old_answered","output":"ok"}
	]}`)

	got := normalizeCodexCallIDs(body)
	if string(got) != string(body) {
		t.Fatalf("existing replay items changed before replay recovery: %s", got)
	}
}

func TestNormalizeCodexCallIDsReplayCompatibility(t *testing.T) {
	for _, tc := range []struct {
		name      string
		body      string
		unchanged bool
	}{
		{"new conversation", `{"input":[{"role":"user","content":"hello"}]}`, true},
		{"existing pair", `{"input":[{"type":"function_call","call_id":"known","namespace":"functions","name":"lookup","arguments":"{}"},{"type":"function_call_output","call_id":"known","output":"saved result"}]}`, true},
		{"output for later replay recovery", `{"input":[{"type":"function_call_output","call_id":"known","output":"saved result"}]}`, true},
		{"legacy repair", `{"input":[{"type":"function_call_output","output":"orphan before"},{"type":"function_call","name":"lookup","arguments":"{}"},{"type":"function_call_output","output":"saved result"},{"type":"custom_tool_call_output","output":"orphan after"}]}`, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			body := []byte(tc.body)
			got := normalizeCodexCallIDs(body)
			if string(body) != tc.body {
				t.Fatal("original request buffer was modified")
			}
			if tc.unchanged && string(got) != tc.body {
				t.Fatalf("valid request changed: %s", got)
			}
			if again := normalizeCodexCallIDs(got); string(again) != string(got) {
				t.Fatalf("normalization is not idempotent: %s -> %s", got, again)
			}
			if !tc.unchanged {
				items := gjson.GetBytes(got, "input").Array()
				if len(items) != 2 || items[0].Get("call_id").String() == "" || items[0].Get("call_id").String() != items[1].Get("call_id").String() || items[1].Get("output").String() != "saved result" {
					t.Fatalf("legacy paired history was not preserved: %s", got)
				}
			}
		})
	}
}

func TestCodexLegacyReplayContinuesThroughHTTP(t *testing.T) {
	for _, mode := range []string{"execute", "stream", "compact"} {
		for _, apiKey := range []bool{false, true} {
			t.Run(fmt.Sprintf("%s/api-key=%v", mode, apiKey), func(t *testing.T) {
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					body, err := io.ReadAll(r.Body)
					if err != nil {
						t.Error(err)
						w.WriteHeader(http.StatusBadRequest)
						return
					}
					calls := map[string]bool{}
					outputs := 0
					for _, item := range gjson.GetBytes(body, "input").Array() {
						switch item.Get("type").String() {
						case "function_call":
							calls[item.Get("call_id").String()] = true
						case "function_call_output":
							id := item.Get("call_id").String()
							if id == "" || !calls[id] {
								t.Errorf("unmatched output reached upstream: %s", body)
								w.WriteHeader(http.StatusBadRequest)
								return
							}
							outputs++
						}
					}
					if outputs != 2 {
						t.Errorf("paired history lost: %s", body)
					}
					if mode == "compact" {
						w.Header().Set("Content-Type", "application/json")
						_, _ = io.WriteString(w, `{"id":"compact_legacy","object":"response.compaction","output":[]}`)
					} else {
						w.Header().Set("Content-Type", "text/event-stream")
						_, _ = fmt.Fprintf(w, "data: %s\n\n", codexCompletedEventBody)
					}
				}))
				defer server.Close()
				auth := apiServiceTestAuth(server.URL)
				if apiKey {
					auth.Attributes["api_key"] = "test"
				}
				req, opts := codexTestRequest()
				if mode == "compact" {
					opts.Alt = "responses/compact"
				}
				executor := NewCodexExecutor(&config.Config{})
				for round := 0; round < 2; round++ {
					req.Payload = []byte(fmt.Sprintf(`{"model":"gpt-5.6-terra","input":[{"type":"function_call_output","output":"legacy orphan"},{"type":"function_call","name":"lookup","namespace":"functions","arguments":"{}"},{"type":"function_call_output","output":"legacy paired result"},{"type":"function_call","call_id":"known","name":"lookup","arguments":"{}"},{"type":"function_call_output","call_id":"known","output":"new paired result"},{"role":"user","content":"continue %d"}]}`, round))
					ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
					if mode == "stream" {
						result, err := executor.ExecuteStream(ctx, auth, req, opts)
						if err != nil {
							cancel()
							t.Fatal(err)
						}
						for chunk := range result.Chunks {
							if chunk.Err != nil {
								t.Error(chunk.Err)
							}
						}
					} else if _, err := executor.Execute(ctx, auth, req, opts); err != nil {
						cancel()
						t.Fatal(err)
					}
					cancel()
				}
			})
		}
	}
}

func TestCodexHTTPAndCompactPreserveReplayNamespaces(t *testing.T) {
	for _, compact := range []bool{false, true} {
		for _, apiKey := range []bool{false, true} {
			t.Run(fmt.Sprintf("compact=%v/api-key=%v", compact, apiKey), func(t *testing.T) {
				captured := make(chan []byte, 1)
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					body, err := io.ReadAll(r.Body)
					if err != nil {
						t.Error(err)
						return
					}
					captured <- body
					if compact {
						w.Header().Set("Content-Type", "application/json")
						_, _ = w.Write([]byte(`{"id":"resp-compact","output":[]}`))
					} else {
						w.Header().Set("Content-Type", "text/event-stream")
						_, _ = fmt.Fprintf(w, "data: %s\n\n", codexCompletedEventBody)
					}
				}))
				defer server.Close()
				auth := apiServiceTestAuth(server.URL)
				if apiKey {
					auth.Attributes["api_key"] = "test"
				}
				req, opts := codexTestRequest()
				req.Payload = namespaceReplayContractPayload()
				if compact {
					opts.Alt = "responses/compact"
				}
				ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
				defer cancel()
				if _, err := NewCodexExecutor(&config.Config{}).Execute(ctx, auth, req, opts); err != nil {
					t.Fatal(err)
				}
				assertNamespaceReplayContract(t, <-captured)
			})
		}
	}
}

func TestCodexWebsocketPreservesReplayNamespacesForAllAuthKinds(t *testing.T) {
	for _, apiKey := range []bool{false, true} {
		captured := make(chan []byte, 1)
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			conn, err := (&websocket.Upgrader{}).Upgrade(w, r, nil)
			if err != nil {
				t.Error(err)
				return
			}
			defer conn.Close()
			if err := conn.SetReadDeadline(time.Now().Add(5 * time.Second)); err != nil {
				t.Error(err)
				return
			}
			_, body, err := conn.ReadMessage()
			if err != nil {
				t.Error(err)
				return
			}
			captured <- body
			_ = conn.WriteMessage(websocket.TextMessage, []byte(codexCompletedEventBody))
		}))
		auth := apiServiceTestAuth(server.URL)
		if apiKey {
			auth.Attributes["api_key"] = "test"
		}
		req, opts := codexTestRequest()
		req.Payload = namespaceReplayContractPayload()
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		_, err := NewCodexWebsocketsExecutor(&config.Config{}).Execute(ctx, auth, req, opts)
		cancel()
		server.Close()
		if err != nil {
			t.Fatal(err)
		}
		assertNamespaceReplayContract(t, <-captured)
	}
}

func namespaceReplayContractPayload() []byte {
	return []byte(`{"model":"gpt-5.6-terra","instructions":"  Keep native instructions exactly.\n","input":[{"type":"function_call","call_id":"c1","name":"lookup","namespace":"functions","arguments":"{}"},{"type":"function_call_output","call_id":"c1","output":"ok"},{"type":"message","role":"user","namespace":"client-extension","content":"hello"}]}`)
}

func assertNamespaceReplayContract(t *testing.T, body []byte) {
	t.Helper()
	if got := gjson.GetBytes(body, "instructions").String(); got != "  Keep native instructions exactly.\n" {
		t.Fatalf("native instructions changed before upstream: %s", body)
	}
	if gjson.GetBytes(body, "input.0.call_id").String() != "c1" {
		t.Fatalf("existing call ID changed before upstream: %s", body)
	}
	if gjson.GetBytes(body, "input.0.namespace").String() != "functions" ||
		gjson.GetBytes(body, "input.2.namespace").String() != "client-extension" ||
		gjson.GetBytes(body, "input.1.call_id").String() != "c1" {
		t.Fatalf("replay metadata changed before upstream: %s", body)
	}
}
