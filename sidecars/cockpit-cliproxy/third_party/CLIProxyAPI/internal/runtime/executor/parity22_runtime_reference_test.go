package executor

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"reflect"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

// Capture the real executor's final HTTP request, after translation, options,
// identity preparation and Lite mode selection. No external endpoint is used.
func TestParity22RuntimeReference(t *testing.T) {
	wire := make([]map[string]any, 0)
	for _, tc := range []struct {
		name   string
		stream bool
		body   string
	}{
		{"summary_stream", true, `{"model":"gpt-5.6-sol","input":[],"stream":true,"stream_options":{"reasoning_summary_delivery":"sequential_cutoff","include_usage":true}}`},
		{"summary_buffered", false, `{"model":"gpt-5.6-sol","input":[],"stream":false,"stream_options":{"reasoning_summary_delivery":"sequential_cutoff","include_usage":true}}`},
		{"metadata_lite_stream", true, `{"model":"gpt-5.6-sol","input":[],"stream":true,"client_metadata":{"ws_request_header_x_openai_internal_codex_responses_lite":true},"tools":[{"type":"web_search"},{"type":"function","name":"fixture","parameters":{"type":"object","properties":{}}}]}`},
		{"metadata_lite_buffered", false, `{"model":"gpt-5.6-sol","input":[],"stream":false,"client_metadata":{"ws_request_header_x_openai_internal_codex_responses_lite":" True "},"tools":[{"type":"web_search"},{"type":"function","name":"fixture","parameters":{"type":"object","properties":{}}}]}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			captured := make(chan map[string]any, 1)
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				data, err := io.ReadAll(r.Body)
				if err != nil {
					t.Error(err)
					return
				}
				var body map[string]any
				if err := json.Unmarshal(data, &body); err != nil {
					t.Error(err)
					return
				}
				captured <- map[string]any{"stream_options": body["stream_options"], "lite_header": r.Header.Get(codexResponsesLiteHeaderName), "parallel_tool_calls": body["parallel_tool_calls"], "tools": body["tools"]}
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"fixture-response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":0,\"output_tokens\":0,\"total_tokens\":0}}}\n\n")
			}))
			defer server.Close()
			cfg := &config.Config{}
			cfg.DisableImageGeneration = config.DisableImageGenerationAll
			e := NewCodexExecutor(cfg)
			auth := &cliproxyauth.Auth{ID: "parity22", Provider: "codex", Attributes: map[string]string{"base_url": server.URL}, Metadata: map[string]any{"access_token": "fixture-only"}}
			req := cliproxyexecutor.Request{Model: "gpt-5.6-sol", Payload: []byte(tc.body)}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, ResponseFormat: sdktranslator.FormatOpenAIResponse, Stream: tc.stream}
			if tc.stream {
				result, err := e.ExecuteStream(context.Background(), auth, req, opts)
				if err != nil {
					t.Fatal(err)
				}
				for chunk := range result.Chunks {
					if chunk.Err != nil {
						t.Fatal(chunk.Err)
					}
				}
			} else if _, err := e.Execute(context.Background(), auth, req, opts); err != nil {
				t.Fatal(err)
			}
			var got map[string]any
			select {
			case got = <-captured:
			case <-time.After(time.Second):
				t.Fatal("executor did not send a request")
			}
			var input any
			_ = json.Unmarshal([]byte(tc.body), &input)
			wire = append(wire, map[string]any{"name": tc.name, "input": input, "expected": got})
		})
	}
	wrapped := make([]map[string]any, 0)
	for _, raw := range []string{
		`{"type":"error","status":429,"body":{"error":{"type":"usage_limit_reached","code":"usage_limit_reached","message":"fixture quota","resets_in_seconds":60}}}`,
		`{"type":"error","status":429,"body":{"error":{"type":"usage_limit_reached","message":"fixture quota without code","resets_in_seconds":7}}}`,
	} {
		err, ok := parseCodexWebsocketError([]byte(raw))
		if !ok {
			t.Fatal("wrapped error was not recognized")
		}
		status := err.(interface{ StatusCode() int }).StatusCode()
		var input any
		_ = json.Unmarshal([]byte(raw), &input)
		entry := map[string]any{"input": input, "status": status, "retry_after_ms": nil, "credential_scope": err.(interface{ IsCredentialScoped() bool }).IsCredentialScoped()}
		if retry := err.(interface{ RetryAfter() *time.Duration }).RetryAfter(); retry != nil {
			entry["retry_after_ms"] = retry.Milliseconds()
		}
		wrapped = append(wrapped, entry)
	}
	wsInput := `{"type":"response.create","model":"gpt-5.6-sol","input":[],"stream":true,"client_metadata":{"ws_request_header_x_openai_internal_codex_responses_lite":true},"tools":[{"type":"web_search"},{"type":"function","name":"fixture","parameters":{"type":"object","properties":{}}}]}`
	wsCaptured := make(chan map[string]any, 1)
	wsServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := (&websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}).Upgrade(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer conn.Close()
		_, data, err := conn.ReadMessage()
		if err != nil {
			t.Error(err)
			return
		}
		var body map[string]any
		if err := json.Unmarshal(data, &body); err != nil {
			t.Error(err)
			return
		}
		wsCaptured <- map[string]any{"stream_options": body["stream_options"], "lite_header": r.Header.Get(codexResponsesLiteHeaderName), "parallel_tool_calls": body["parallel_tool_calls"], "tools": body["tools"]}
		_ = conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.completed","response":{"id":"fixture-ws","status":"completed","output":[],"usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0}}}`))
	}))
	defer wsServer.Close()
	wsCfg := &config.Config{}
	wsCfg.DisableImageGeneration = config.DisableImageGenerationAll
	wsResult, err := NewCodexWebsocketsExecutor(wsCfg).ExecuteStream(context.Background(), &cliproxyauth.Auth{ID: "parity22-ws", Provider: "codex", Attributes: map[string]string{"base_url": wsServer.URL}, Metadata: map[string]any{"access_token": "fixture-only"}}, cliproxyexecutor.Request{Model: "gpt-5.6-sol", Payload: []byte(wsInput)}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, ResponseFormat: sdktranslator.FormatOpenAIResponse, Stream: true})
	if err != nil {
		t.Fatal(err)
	}
	for chunk := range wsResult.Chunks {
		if chunk.Err != nil {
			t.Fatal(chunk.Err)
		}
	}
	var wsGot map[string]any
	select {
	case wsGot = <-wsCaptured:
	case <-time.After(time.Second):
		t.Fatal("websocket executor did not send a request")
	}
	var wsInputJSON any
	_ = json.Unmarshal([]byte(wsInput), &wsInputJSON)
	actual := map[string]any{"wire": wire, "wrapped_errors": wrapped, "websocket_wire": map[string]any{"input": wsInputJSON, "expected": wsGot}}
	data, err := json.MarshalIndent(actual, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	path := os.Getenv("PARITY22_RUNTIME_REFERENCE_OUT")
	if path != "" {
		if err := os.WriteFile(path, append(data, '\n'), 0600); err != nil {
			t.Fatal(err)
		}
		return
	}
	path = "../../../../../../../src-tauri/src/modules/codex_native_api/fixtures/parity22_runtime_go.json"
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var gotJSON, wantJSON any
	_ = json.Unmarshal(data, &gotJSON)
	if err := json.Unmarshal(want, &wantJSON); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(gotJSON, wantJSON) {
		t.Fatalf("runtime reference changed; recapture actual executor wire with PARITY22_RUNTIME_REFERENCE_OUT: %s", data)
	}
}
