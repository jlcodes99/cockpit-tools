package executor

import (
	"context"
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

// Capture the actual request after schema normalization, Lite hosted-tool
// filtering, and replay injection. A helper-only test cannot verify this order.
func TestCodexWebSearchHistoryTransport(t *testing.T) {
	const history = `{"type":"web_search_call","id":"ws_search_1","status":"completed","action":{"type":"search","query":"test"}}`
	const trigger = `{"type":"compaction_trigger"}`
	for _, transport := range []string{"http execute", "http stream", "websocket execute", "websocket stream"} {
		for _, tc := range []struct {
			name, extra, beforeHistory string
			lite                       bool
			metadataLite               bool
			escapedHistory             bool
			apiKey                     bool
			compact                    bool
			full                       bool
			wantChoice                 string
		}{
			{name: "standard empty tools", extra: `,"tools":[],"tool_choice":"auto"`, wantChoice: "none"},
			{name: "standard escaped history", escapedHistory: true, extra: `,"tools":[],"tool_choice":"auto"`, wantChoice: "none"},
			{name: "lite escaped history", escapedHistory: true, lite: true, extra: `,"tools":[],"tool_choice":"auto"`, wantChoice: "none"},
			{name: "lite empty tools", lite: true, extra: `,"tools":[],"tool_choice":"auto"`, wantChoice: "none"},
			{name: "lite metadata hint", lite: true, metadataLite: true, extra: `,"tools":[],"tool_choice":"auto","client_metadata":{"ws_request_header_x_openai_internal_codex_responses_lite":"true"}`, wantChoice: "none"},
			{name: "lite filtered cached declaration", lite: true, extra: `,"tools":[{"type":"web_search","external_web_access":false}],"tool_choice":"none"`, wantChoice: "none"},
			{name: "caller function auto", lite: true, extra: `,"tools":[{"type":"function","name":"echo","parameters":{"type":"object"}}],"tool_choice":"auto"`, wantChoice: "auto"},
			{name: "caller custom required", lite: true, extra: `,"tools":[{"type":"custom","name":"apply_patch"}],"tool_choice":"required"`, wantChoice: "required"},
			{name: "lite migrated tools", lite: true, beforeHistory: `{"type":"additional_tools","role":"developer","tools":[{"type":"function","name":"echo","parameters":{"type":"object"}}]},`, extra: `,"tool_choice":"auto"`, wantChoice: "auto"},
			{name: "lite image full fallback", lite: true, full: true, extra: `,"tools":[{"type":"image_generation"}],"tool_choice":{"type":"image_generation"}`, wantChoice: `{"type":"image_generation"}`},
			{name: "API key not patched", apiKey: true, extra: `,"tools":[],"tool_choice":"auto"`, wantChoice: "auto"},
			{name: "API key Lite not patched", lite: true, apiKey: true, extra: `,"tools":[],"tool_choice":"auto"`, wantChoice: "auto"},
			{name: "explicit compact not patched", compact: true, extra: `,"tools":[],"tool_choice":"auto"`, wantChoice: "auto"},
		} {
			if tc.compact && strings.HasSuffix(transport, "stream") {
				continue // /responses/compact rejects streaming before any request.
			}
			t.Run(transport+"/"+tc.name, func(t *testing.T) {
				type capturedRequest struct {
					body   []byte
					header http.Header
					path   string
				}
				captured := make(chan capturedRequest, 1)
				upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
				completed := []byte(`{"type":"response.completed","response":{"id":"resp_search_test","object":"response","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}`)
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					if websocket.IsWebSocketUpgrade(r) {
						conn, errUpgrade := upgrader.Upgrade(w, r, nil)
						if errUpgrade != nil {
							t.Errorf("upgrade: %v", errUpgrade)
							return
						}
						defer func() { _ = conn.Close() }()
						_, body, errRead := conn.ReadMessage()
						if errRead != nil {
							t.Errorf("read: %v", errRead)
							return
						}
						captured <- capturedRequest{body, r.Header.Clone(), r.URL.Path}
						if errWrite := conn.WriteMessage(websocket.TextMessage, completed); errWrite != nil {
							t.Errorf("write: %v", errWrite)
						}
						return
					}
					body, errRead := io.ReadAll(r.Body)
					if errRead != nil {
						t.Errorf("read body: %v", errRead)
						return
					}
					captured <- capturedRequest{body, r.Header.Clone(), r.URL.Path}
					if tc.compact {
						w.Header().Set("Content-Type", "application/json")
						_, _ = w.Write([]byte(`{"id":"compact_search_test","object":"response.compaction","output":[]}`))
						return
					}
					w.Header().Set("Content-Type", "text/event-stream")
					_, _ = w.Write(append(append([]byte("data: "), completed...), []byte("\n\n")...))
				}))
				t.Cleanup(server.Close)
				ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
				defer cancel()
				// Passthrough image mode prevents a default tool from changing the
				// empty-tools fixture while retaining explicit image fallback.
				cfg := &config.Config{}
				cfg.DisableImageGeneration = config.DisableImageGenerationPassthrough
				auth := &cliproxyauth.Auth{ID: "search-history-test", Provider: "codex", Attributes: map[string]string{"base_url": server.URL}, Metadata: map[string]any{"access_token": "oauth-test-token"}}
				if tc.apiKey {
					// Even legacy OAuth metadata must not override the real key kind.
					auth.Attributes["api_key"] = "api-test-key"
				}
				opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FromString("openai-response"), ResponseFormat: sdktranslator.FromString("openai-response")}
				if tc.lite && !tc.metadataLite {
					opts.Headers = http.Header{codexResponsesLiteHeaderName: []string{"true"}}
				}
				if tc.compact {
					opts.Alt = "responses/compact"
				}
				searchHistory := history
				if tc.escapedHistory {
					searchHistory = strings.Replace(history, "web_search_call", `web_\u0073earch_call`, 1)
				}
				req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: []byte(`{"model":"gpt-5.4","instructions":"","input":[` + tc.beforeHistory + searchHistory + `,` + trigger + `]` + tc.extra + `}`)}
				var executor cliproxyauth.ProviderExecutor
				if strings.HasPrefix(transport, "websocket") {
					executor = NewCodexWebsocketsExecutor(cfg)
				} else {
					executor = NewCodexExecutor(cfg)
				}
				if strings.HasSuffix(transport, "stream") {
					result, errExecute := executor.ExecuteStream(ctx, auth, req, opts)
					if errExecute != nil {
						t.Fatalf("ExecuteStream: %v", errExecute)
					}
					for chunk := range result.Chunks {
						if chunk.Err != nil {
							t.Fatalf("stream: %v", chunk.Err)
						}
					}
				} else if _, errExecute := executor.Execute(ctx, auth, req, opts); errExecute != nil {
					t.Fatalf("Execute: %v", errExecute)
				}
				var got capturedRequest
				select {
				case got = <-captured:
				case <-ctx.Done():
					t.Fatal("no upstream request captured")
				}
				if choice := gjson.GetBytes(got.body, "tool_choice").String(); choice != tc.wantChoice {
					t.Fatalf("tool_choice=%q, want %q: %s", choice, tc.wantChoice, got.body)
				}
				topSearch, additionalSearch := false, false
				callerTools := 0
				assertTools := func(tools gjson.Result) bool {
					search := false
					for _, tool := range tools.Array() {
						if name := tool.Get("name").String(); name == "echo" || name == "apply_patch" {
							callerTools++
						}
						if tool.Get("type").String() == "web_search" {
							if tool.Get("external_web_access").Type != gjson.False {
								t.Fatalf("search must remain cached-only: %s", got.body)
							}
							search = true
						}
					}
					return search
				}
				topSearch = assertTools(gjson.GetBytes(got.body, "tools"))
				items := gjson.GetBytes(got.body, "input").Array()
				for _, item := range items {
					if item.Get("type").String() == "additional_tools" {
						additionalSearch = assertTools(item.Get("tools")) || additionalSearch
					}
				}
				if tc.apiKey || tc.compact {
					if topSearch || additionalSearch {
						t.Fatalf("patch escaped OAuth /responses scope: %s", got.body)
					}
				} else if tc.lite && !tc.full {
					if topSearch || !additionalSearch || !codexResponsesLiteEnabled(got.header) {
						t.Fatalf("Lite must carry additional declaration and header: %s / %v", got.body, got.header)
					}
				} else if !topSearch || additionalSearch || tc.full && codexResponsesLiteEnabled(got.header) {
					t.Fatalf("full Responses must use top-level declaration: %s / %v", got.body, got.header)
				}
				historyCount := 0
				for _, item := range items {
					if item.Get("type").String() == "web_search_call" {
						historyCount++
					}
				}
				if len(items) < 2 || historyCount != 1 || items[len(items)-1].Get("type").String() != "compaction_trigger" {
					t.Fatalf("history/trigger order lost: %s", got.body)
				}
				if (strings.HasPrefix(tc.name, "caller ") || tc.name == "lite migrated tools") && callerTools != 1 {
					t.Fatalf("caller tool declaration lost or duplicated: %s", got.body)
				}
				if tc.compact && got.path != "/responses/compact" {
					t.Fatalf("compact went to %s", got.path)
				}
			})
		}
	}
}
