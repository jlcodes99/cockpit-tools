package executor

import (
	"context"
	"fmt"
	"io"
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

func TestXAIWebSearchCompatibilityTransports(t *testing.T) {
	for _, transport := range []string{"http", "sse", "ws"} {
		t.Run(transport, func(t *testing.T) {
			cfg := &config.Config{Payload: config.PayloadConfig{Override: []config.PayloadRule{{Models: []config.PayloadModelRule{{Name: "grok-4.6"}}, Params: map[string]any{"stop": []string{"configured"}, "instructions": "configured"}}}}}
			upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
			inspect := func(body []byte) string {
				if gjson.GetBytes(body, "stop.0").String() != "configured" || gjson.GetBytes(body, "instructions").String() != "configured" {
					t.Errorf("final payload rules were overwritten: %s", body)
				}
				alias := ""
				for _, tool := range gjson.GetBytes(body, "tools").Array() {
					if tool.Get("name").String() == xaiClientWebSearchAlias+"_1" {
						alias = tool.Get("name").String()
					}
					if tool.Get("type").String() == "tool_search" {
						t.Error("client tool_search leaked upstream")
					}
					if tool.Get("type").String() != "web_search" {
						continue
					}
					for _, key := range []string{"external_web_access", "search_context_size"} {
						if tool.Get(key).Exists() {
							t.Errorf("unsupported %s leaked upstream", key)
						}
					}
					if tool.Get("filters.allowed_domains.0").String() != "example.com" || tool.Get("user_location.country").String() != "US" || tool.Get("search_content_types.0").String() != "text" {
						t.Errorf("accepted options lost: %s", tool.Raw)
					}
				}
				if alias == "" || gjson.GetBytes(body, "input.0.name").String() != alias || gjson.GetBytes(body, "tool_choice.name").String() != alias {
					t.Errorf("client search alias/history/choice inconsistent: %s", body)
				}
				return alias
			}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				var body []byte
				var conn *websocket.Conn
				if transport == "ws" {
					var err error
					conn, err = upgrader.Upgrade(w, r, nil)
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
					w.Header().Set("Content-Type", "text/event-stream")
				}
				alias := inspect(body)
				item := fmt.Sprintf(`{"type":"function_call","id":"fc_web","call_id":"call_web","name":%q,"arguments":"{\"query\":\"test\"}"}`, alias)
				events := []string{
					`{"type":"response.output_item.added","output_index":0,"item":` + item + `}`,
					`{"type":"response.output_item.done","output_index":0,"item":` + item + `}`,
					`{"type":"response.completed","response":{"id":"resp_web","status":"completed","output":[` + item + `]}}`,
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
			body := []byte(`{"model":"grok-4.6","tools":[{"type":"tool_search","execution":"client"},{"type":"web_search","external_web_access":false,"search_context_size":"low","filters":{"allowed_domains":["example.com"]},"user_location":{"type":"approximate","country":"US"},"search_content_types":["text"]},{"type":"function","name":"web_search","parameters":{"type":"object"}},{"type":"function","name":"clientfn_web_search","parameters":{"type":"object"}}],"tool_choice":{"type":"function","name":"web_search"},"input":[{"type":"function_call","name":"web_search","call_id":"old","arguments":"{}"},{"type":"function_call_output","call_id":"old","output":"result"}]}`)
			auth := &cliproxyauth.Auth{ID: "web-search-" + transport, Provider: "xai", Attributes: map[string]string{"base_url": server.URL, "api_key": "test"}}
			req := cliproxyexecutor.Request{Model: "grok-4.6", Payload: body}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, ResponseFormat: sdktranslator.FormatOpenAIResponse}
			var result []byte
			if transport == "http" {
				response, err := NewXAIExecutor(cfg).Execute(t.Context(), auth, req, opts)
				if err != nil {
					t.Fatal(err)
				}
				result = response.Payload
			} else {
				var stream *cliproxyexecutor.StreamResult
				var err error
				if transport == "ws" {
					stream, err = NewXAIWebsocketsExecutor(cfg).ExecuteStream(cliproxyexecutor.WithDownstreamWebsocket(t.Context()), auth, req, opts)
				} else {
					stream, err = NewXAIExecutor(cfg).ExecuteStream(t.Context(), auth, req, opts)
				}
				if err != nil {
					t.Fatal(err)
				}
				for chunk := range stream.Chunks {
					if chunk.Err != nil {
						t.Fatal(chunk.Err)
					}
					for _, line := range strings.Split(string(chunk.Payload), "\n") {
						line = strings.TrimSpace(strings.TrimPrefix(line, "data:"))
						if gjson.Get(line, "type").String() == "response.completed" {
							result = []byte(gjson.Get(line, "response").Raw)
						}
					}
				}
			}
			if gjson.GetBytes(result, "output.0.name").String() != "web_search" || gjson.GetBytes(result, "output.0.call_id").String() != "call_web" {
				t.Fatalf("client search identity lost: %s", result)
			}
		})
	}
}

func TestXAIForcedHostedWebSearchCannotInjectOtherTools(t *testing.T) {
	for _, choice := range []string{`{"type":"web_search"}`, `{"type":"allowed_tools","mode":"required","tools":[{"type":"web_search"}]}`, `{"type":"allowed_tools","mode":"auto","tools":[{"type":"web_search"}]}`} {
		body := []byte(`{"model":"grok-4.6","input":[],"tools":[{"type":"web_search","external_web_access":false,"search_context_size":"low"},{"type":"function","name":"lookup"}],"tool_choice":` + choice + `}`)
		prepared, err := NewXAIExecutor(&config.Config{XAI: config.XAIConfig{InjectXSearch: true}}).prepareResponsesRequest(context.Background(), cliproxyexecutor.Request{Model: "grok-4.6", Payload: body}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse}, true)
		if err != nil {
			t.Fatal(err)
		}
		if gjson.GetBytes(prepared.body, "tools.#").Int() != 1 || gjson.GetBytes(prepared.body, "tools.0.type").String() != "web_search" || gjson.GetBytes(prepared.body, "tool_choice").Type != gjson.String {
			t.Fatalf("forced hosted search broadened: %s", prepared.body)
		}
	}
}

func TestXAIClientWebSearchHistorySurvivesAliasChange(t *testing.T) {
	body := []byte(`{"input":[{"type":"function_call","name":"clientfn_web_search","call_id":"old","arguments":"{}"},{"type":"function_call_output","call_id":"old","output":"literal clientfn_web_search"}]}`)
	restored := restoreXAIClientWebSearchHistory(body, "clientfn_web_search")
	replayed := aliasXAIClientWebSearchInput(restored, "clientfn_web_search_1", nil)
	if gjson.GetBytes(replayed, "input.0.name").String() != "clientfn_web_search_1" || gjson.GetBytes(replayed, "input.1.output").String() != "literal clientfn_web_search" {
		t.Fatalf("alias remapping changed history identity or output: %s", replayed)
	}
}
