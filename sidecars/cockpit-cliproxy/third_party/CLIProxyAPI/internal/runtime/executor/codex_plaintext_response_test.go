package executor

import (
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gorilla/websocket"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestCodexPlaintextMarkersUseFinalTransmittedDeclarations(t *testing.T) {
	for _, mode := range []string{"http", "compact", "http-stream", "http-buffered", "ws", "ws-stream", "ws-buffered"} {
		for _, tc := range []struct {
			name, declaration                string
			additional, continuity, disabled bool
			want                             map[string]bool
		}{
			{name: "optimized top-level", declaration: `true`, want: map[string]bool{"spawn_agent": true, "send_message": true, "followup_task": true}},
			{name: "optimized additional tools", declaration: `true`, additional: true, want: map[string]bool{"spawn_agent": true, "send_message": true, "followup_task": true}},
			{name: "canonical keeps actual optimized alias", declaration: `true`, continuity: true, want: map[string]bool{"spawn_agent": true, "send_message": true, "followup_task": true}},
			{name: "canonical restores encryption", declaration: `true`, additional: true, continuity: true},
			{name: "canonical restores mixed schemas", declaration: `mixed`, additional: true, continuity: true, want: map[string]bool{"spawn_agent": true}},
			{name: "missing declarations", declaration: `missing`},
			{name: "disabled with plaintext schema", declaration: `false`, disabled: true},
		} {
			t.Run(mode+"/"+tc.name, func(t *testing.T) {
				captured := make(chan []byte, 1)
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					var body []byte
					var conn *websocket.Conn
					if strings.HasPrefix(mode, "ws") {
						var err error
						conn, err = (&websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}).Upgrade(w, r, nil)
						if err != nil {
							t.Error(err)
							return
						}
						defer func() { _ = conn.Close() }()
						_, body, err = conn.ReadMessage()
						if err != nil {
							t.Error(err)
							return
						}
					} else {
						body, _ = io.ReadAll(r.Body)
					}
					captured <- body
					namespace := gjson.GetBytes(body, "tools.0.name").String()
					if tc.additional {
						namespace = gjson.GetBytes(body, "input.0.tools.0.name").String()
					}
					if namespace == "" {
						namespace = "collaboration"
					}
					items, events := plaintextMarkerTestEvents(namespace)
					if mode == "compact" {
						w.Header().Set("Content-Type", "application/json")
						_, _ = io.WriteString(w, `{"id":"compact-1","object":"response.compaction","output":[`+items+`]}`)
						return
					}
					w.Header().Set("Content-Type", "text/event-stream")
					for _, event := range events {
						if conn != nil {
							_ = conn.WriteMessage(websocket.TextMessage, []byte(event))
						} else {
							_, _ = fmt.Fprintf(w, "data: %s\n\n", event)
						}
					}
				}))
				defer server.Close()
				request := plaintextMarkerTestRequest(tc.declaration, tc.additional)
				ctx := codexSpawnAgentTestContext()
				var hook *codexContinuityHook
				if tc.continuity {
					hook = &codexContinuityHook{}
					ctx = codexContinuityContext(t, hook, request, mode == "compact")
				}
				cfg := codexContinuityConfig()
				cfg.Codex.OptimizeMultiAgentV2 = !tc.disabled
				cfg.Codex.StreamBootstrapBuffering = strings.Contains(mode, "buffered")
				var executor cliproxyauth.ProviderExecutor = NewCodexExecutor(cfg)
				if strings.HasPrefix(mode, "ws") {
					executor = NewCodexWebsocketsExecutor(cfg)
				}
				opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Headers: http.Header{"User-Agent": {"codex-tui/0.145.0"}}}
				if mode == "compact" {
					opts.Alt = "responses/compact"
				}
				req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: request}
				auth := codexRecoveryTestAuth(server.URL, "plaintext-marker-oauth")
				var response []byte
				stream := strings.Contains(mode, "stream") || strings.Contains(mode, "buffered")
				if stream {
					result, err := executor.ExecuteStream(ctx, auth, req, opts)
					if err != nil {
						t.Fatal(err)
					}
					for chunk := range result.Chunks {
						if chunk.Err != nil {
							t.Fatal(chunk.Err)
						}
						response = append(response, chunk.Payload...)
						response = append(response, '\n')
					}
				} else {
					result, err := executor.Execute(ctx, auth, req, opts)
					if err != nil {
						t.Fatal(err)
					}
					response = result.Payload
				}
				actual := <-captured
				if tc.continuity && tc.additional && gjson.GetBytes(actual, "input.0.tools.0.tools.1.parameters.properties.message.encrypted").Raw != "true" {
					t.Fatalf("fixture failed to restore canonical encrypted declaration: %s", actual)
				}
				assertPlaintextMarkerTestResponse(t, response, tc.want, stream)
				if hook != nil {
					namespace := gjson.GetBytes(actual, "tools.0.name").String()
					if tc.additional {
						namespace = gjson.GetBytes(actual, "input.0.tools.0.name").String()
					}
					hook.mu.Lock()
					defer hook.mu.Unlock()
					observed := 0
					for _, event := range hook.events {
						root := gjson.ParseBytes(event)
						items := root.Get("response.output").Array()
						if root.Get("output").IsArray() {
							items = root.Get("output").Array()
						}
						if root.Get("item").IsObject() {
							items = append(items, root.Get("item"))
						}
						for _, item := range items {
							name := item.Get("name").String()
							if marked := item.Get("encrypted_function_args").Raw == "[]"; marked != tc.want[name] {
								t.Fatalf("Observe persisted wrong metadata: %s", item.Raw)
							}
							if item.Get("namespace").String() != namespace || item.Get("arguments").String() != `{"message":"gAAAA-upstream-value"}` || item.Get("id").String() == "" || item.Get("call_id").String() == "" {
								t.Fatalf("Observe projection changed wire identity/arguments: %s", item.Raw)
							}
							observed++
						}
					}
					if observed < 3 {
						t.Fatalf("missing output observations: %d", observed)
					}
				}
			})
		}
	}
}

func plaintextMarkerTestRequest(declaration string, additional bool) []byte {
	var tools []string
	for index, name := range []string{"spawn_agent", "send_message", "followup_task"} {
		flag := declaration
		if declaration == "mixed" {
			flag = []string{"false", "true", `"unknown"`}[index]
		}
		tools = append(tools, fmt.Sprintf(`{"type":"function","name":%q,"parameters":{"type":"object","properties":{"message":{"type":"string","encrypted":%s}}}}`, name, flag))
	}
	namespace := `{"type":"namespace","name":"collaboration","tools":[` + strings.Join(tools, ",") + `]}`
	if declaration == "missing" {
		return []byte(`{"model":"gpt-5.4","input":[]}`)
	}
	if additional {
		return []byte(`{"model":"gpt-5.4","input":[{"type":"additional_tools","role":"developer","tools":[` + namespace + `]}]}`)
	}
	return []byte(`{"model":"gpt-5.4","tools":[` + namespace + `],"input":[]}`)
}

func plaintextMarkerTestEvents(namespace string) (string, []string) {
	var items, events []string
	for i, name := range []string{"spawn_agent", "send_message", "followup_task"} {
		item := fmt.Sprintf(`{"id":"fc_%d","type":"function_call","name":%q,"namespace":%q,"call_id":"call_%d","arguments":"{\"message\":\"gAAAA-upstream-value\"}"}`, i, name, namespace, i)
		items = append(items, item)
		for _, event := range []string{"response.output_item.added", "response.output_item.done"} {
			events = append(events, fmt.Sprintf(`{"type":%q,"output_index":%d,"item":%s}`, event, i, item))
		}
	}
	joined := strings.Join(items, ",")
	events = append(events, `{"type":"response.completed","response":{"id":"resp-1","status":"completed","output":[`+joined+`]}}`)
	return joined, events
}

func assertPlaintextMarkerTestResponse(t *testing.T, response []byte, want map[string]bool, stream bool) {
	t.Helper()
	check := func(item gjson.Result) {
		t.Helper()
		name := item.Get("name").String()
		if marker := item.Get("encrypted_function_args").Raw == "[]"; marker != want[name] {
			t.Fatalf("%s marker=%t want=%t: %s", name, marker, want[name], item.Raw)
		}
		if item.Get("arguments").String() != `{"message":"gAAAA-upstream-value"}` {
			t.Fatalf("upstream arguments changed: %s", item.Raw)
		}
	}
	if !stream {
		items := gjson.GetBytes(response, "output").Array()
		if len(items) != 3 {
			t.Fatalf("missing output items: %s", response)
		}
		for _, item := range items {
			check(item)
		}
		return
	}
	seen := map[string]bool{}
	for _, line := range strings.Split(string(response), "\n") {
		if !strings.HasPrefix(line, "data:") {
			continue
		}
		data := gjson.Parse(strings.TrimSpace(strings.TrimPrefix(line, "data:")))
		switch event := data.Get("type").String(); event {
		case "response.output_item.added", "response.output_item.done":
			check(data.Get("item"))
			seen[event] = true
		case "response.completed":
			for _, item := range data.Get("response.output").Array() {
				check(item)
			}
			seen[event] = true
		}
	}
	if len(seen) != 3 {
		t.Fatalf("missing stream event shapes: %v; %s", seen, response)
	}
}
