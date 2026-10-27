package executor

import (
	"context"
	"fmt"
	"io"
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

type rebuiltToolsHook struct{ canonical []byte }

func (h *rebuiltToolsHook) Prepare(_ context.Context, r cliproxyexecutor.ContinuityPrepareRequest) (cliproxyexecutor.ContinuityPrepareResponse, error) {
	return cliproxyexecutor.ContinuityPrepareResponse{Ticket: "tool-test", Body: r.Body, Headers: r.Headers}, nil
}
func (h *rebuiltToolsHook) Bind(_ context.Context, r cliproxyexecutor.ContinuityBindRequest) (cliproxyexecutor.ContinuityBindResponse, error) {
	return cliproxyexecutor.ContinuityBindResponse{Ticket: r.Ticket, AttemptID: r.AttemptID, Body: h.canonical, Headers: r.Headers}, nil
}
func (*rebuiltToolsHook) Observe(_ context.Context, r cliproxyexecutor.ContinuityObserveRequest) (cliproxyexecutor.ContinuityObserveResponse, error) {
	return cliproxyexecutor.ContinuityObserveResponse{NextSequence: r.Sequence + 1}, nil
}
func (*rebuiltToolsHook) Finish(context.Context, cliproxyexecutor.ContinuityFinishRequest) error {
	return nil
}

func TestCodexExecutorContinuityRebuiltTools(t *testing.T) {
	testCodexExecutorContinuityRebuiltTools(t, false)
}

func TestCodexExecutorContinuityRebuiltAgentMessageIDs(t *testing.T) {
	testCodexExecutorContinuityRebuiltTools(t, true)
}

func testCodexExecutorContinuityRebuiltTools(t *testing.T, receipt bool) {
	payload := []byte(`{"model":"test","input":[{"type":"additional_tools","role":"developer","tools":[{"type":"namespace","name":"collaboration","description":"test","tools":[{"type":"function","name":"spawn_agent","parameters":{"properties":{"message":{"type":"string","encrypted":true}}}}]}]},{"role":"user","content":"task"}]}`)
	if receipt {
		payload = []byte(strings.TrimSuffix(string(payload), `]}`) + `,{"type":"agent_message","id":"amsg_01a15d7a-297f-7ac2-9b3b-e4d2c36571da","author":"/root/child","recipient":"/root","content":[{"type":"input_text","text":"verified child receipt"}]}]}`)
	}
	for _, mode := range []string{"stream", "execute", "compact"} {
		t.Run(mode, func(t *testing.T) {
			var sent []byte
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				sent, _ = io.ReadAll(r.Body)
				for _, item := range gjson.GetBytes(sent, "input").Array() {
					if id := item.Get("id"); item.Get("type").String() == "message" && id.Exists() && !strings.HasPrefix(id.String(), "msg") {
						w.Header().Set("Content-Type", "application/json")
						w.WriteHeader(http.StatusBadRequest)
						fmt.Fprintf(w, `{"error":{"code":"invalid_id_prefix","message":"ordinary message ID must begin with msg; got %s"}}`, id.String())
						return
					}
				}
				namespace := gjson.GetBytes(sent, "input.0.tools.0.name").String()
				output := fmt.Sprintf(`{"type":"function_call","name":"spawn_agent","namespace":%q,"arguments":"{\"message\":\"plain task\"}","call_id":"tool-1"}`, namespace)
				if mode == "compact" {
					w.Header().Set("Content-Type", "application/json")
					fmt.Fprintf(w, `{"id":"response-1","object":"response.compaction","output":[%s]}`, output)
					return
				}
				w.Header().Set("Content-Type", "text/event-stream")
				fmt.Fprintf(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-1\",\"status\":\"completed\",\"output\":[%s]}}\n\n", output)
			}))
			defer server.Close()
			cfg := &config.Config{Codex: config.CodexConfig{OptimizeMultiAgentV2: true}, CodexKey: []config.CodexKey{{APIKey: "test", BaseURL: server.URL, Models: []config.CodexModel{{Name: "test", IsCompat: true}}}}}
			auth := &cliproxyauth.Auth{ID: "test-auth", Provider: "codex", Attributes: map[string]string{"api_key": "test", "base_url": server.URL}}
			ctx, _, err := cliproxyexecutor.PrepareContinuity(cliproxyexecutor.WithContinuityHook(codexSpawnAgentTestContext(), &rebuiltToolsHook{canonical: payload}), cliproxyexecutor.ContinuityPrepareRequest{Body: payload})
			if err != nil {
				t.Fatal(err)
			}
			e := NewCodexExecutor(cfg)
			request := cliproxyexecutor.Request{Model: "test", Payload: payload}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Headers: http.Header{"User-Agent": []string{"Codex Desktop/0.163.0"}}}
			var result []byte
			if mode == "stream" {
				stream, err := e.ExecuteStream(ctx, auth, request, opts)
				if err != nil {
					t.Fatal(err)
				}
				for chunk := range stream.Chunks {
					if chunk.Err != nil {
						t.Fatal(chunk.Err)
					}
					result = append(result, chunk.Payload...)
				}
			} else {
				if mode == "compact" {
					opts.Alt = "responses/compact"
				}
				response, err := e.Execute(ctx, auth, request, opts)
				if err != nil {
					t.Fatal(err)
				}
				result = response.Payload
			}
			if gjson.GetBytes(sent, "input.0.tools.0.name").String() != "collaboration-optimize" || gjson.GetBytes(sent, "input.0.tools.0.tools.0.parameters.properties.message.encrypted").Exists() {
				t.Fatalf("canonical encrypted tool reached upstream: %s", sent)
			}
			if !containsPlaintextToolMarker(result) {
				t.Fatalf("plaintext call not marked in client response: %s", result)
			}
			if receipt && (gjson.GetBytes(sent, "input.2.id").Exists() || gjson.GetBytes(sent, "input.2.content.0.text").String() != "verified child receipt") {
				t.Fatalf("child receipt lost or private ID retained: %s", sent)
			}
		})
	}
}

func containsPlaintextToolMarker(payload []byte) bool {
	// Execute responses are JSON; stream chunks can include SSE data prefixes.
	return strings.Contains(string(payload), `"encrypted_function_args":[]`)
}
