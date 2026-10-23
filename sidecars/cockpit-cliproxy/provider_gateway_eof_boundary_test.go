package main

import (
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

// Parse real SSE frames: counting event names alone misses glued JSON/events.
func gatewayReviewSSEEvents(t *testing.T, output string) []gjson.Result {
	t.Helper()
	var events []gjson.Result
	for _, frame := range strings.Split(output, "\n\n") {
		if strings.TrimSpace(frame) == "" {
			continue
		}
		var eventName, data string
		for _, line := range strings.Split(frame, "\n") {
			if strings.HasPrefix(line, "event:") {
				if eventName != "" {
					t.Fatalf("events were not separated: %q", frame)
				}
				eventName = strings.TrimSpace(strings.TrimPrefix(line, "event:"))
			} else if strings.HasPrefix(line, "data:") {
				data += strings.TrimSpace(strings.TrimPrefix(line, "data:"))
			}
		}
		if eventName == "" || !gjson.Valid(data) || gjson.Get(data, "type").String() != eventName {
			t.Fatalf("invalid SSE event: %q", frame)
		}
		events = append(events, gjson.Parse(data))
	}
	return events
}

func TestProviderGatewayEOFBoundaryAndFraming(t *testing.T) {
	gin.SetMode(gin.TestMode)
	const text = `data: {"id":"c","choices":[{"index":0,"delta":{"content":"answer"}}]}` + "\n\n"
	const stop = `data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}` + "\n\n"
	const choices = `data: {"id":"c","choices":[{"index":0,"delta":{"content":"done"}},{"index":1,"delta":{"content":"unfinished"}}]}` + "\n\n"
	const partialTool = `data: {"id":"c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"lookup","arguments":"{\"x\":"}}]}}]}` + "\n\n"
	for _, translated := range []bool{false, true} {
		for _, tc := range []struct{ name, stream, terminal, request string }{
			{"unfinished second choice", choices + stop, "response.failed", `{}`},
			{"all choices finished", choices + stop + `data: {"choices":[{"index":1,"delta":{},"finish_reason":"stop"}]}` + "\n\n", "response.completed", `{}`},
			{"second choice continues", choices + stop + `data: {"choices":[{"index":1,"delta":{"content":" more"}}]}` + "\n\n" + `data: {"choices":[{"index":1,"delta":{},"finish_reason":"stop"}]}` + "\n\n", "response.completed", `{}`},
			{"partial choice remains incomplete", choices + `data: {"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}` + "\n\n" + `data: {"choices":[{"index":1,"delta":{},"finish_reason":"stop"}]}` + "\n\n", "response.incomplete", `{}`},
			{"unfinished function arguments", partialTool + `data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}` + "\n\n", "response.failed", `{}`},
			{"unfinished function arguments with DONE", partialTool + `data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}` + "\n\ndata: [DONE]\n\n", "response.failed", `{}`},
			{"length permits partial arguments", partialTool + `data: {"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}` + "\n\n", "response.incomplete", `{}`},
			{"custom freeform arguments", `data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_custom","function":{"name":"patch","arguments":"freeform patch"}}]}}]}` + "\n\n" + `data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}` + "\n\n", "response.completed", `{"tools":[{"type":"custom","name":"patch"}]}`},
			{"bare upstream error", text + stop + `{"error":{"message":"upstream failed","code":"server_error"}}` + "\n", "response.failed", `{}`},
			{"valid EOF framing", text + stop, "response.completed", `{}`},
		} {
			name := tc.name
			if translated {
				name = "translated/" + name
			}
			t.Run(name, func(t *testing.T) {
				w := httptest.NewRecorder()
				c, _ := gin.CreateTestContext(w)
				c.Request = httptest.NewRequest("POST", "/v1/responses", nil)
				if translated {
					(&relayServer{}).writeProviderGatewayTranslatedChatStream(c, strings.NewReader(tc.stream), "fixture", []byte(tc.request), []byte(tc.request), sdktranslator.FormatOpenAIResponse, "")
				} else {
					(&relayServer{}).writeProviderGatewayChatStream(c, strings.NewReader(tc.stream), "fixture", []byte(tc.request), []byte(tc.request), false)
				}
				terminals := 0
				texts := make(map[string]string)
				ended := make(map[string]bool)
				for _, event := range gatewayReviewSSEEvents(t, w.Body.String()) {
					switch event.Get("type").String() {
					case "response.output_text.delta":
						id := event.Get("item_id").String()
						if ended[id] {
							t.Fatalf("text arrived after its item was finalized: %s", event.Raw)
						}
						texts[id] += event.Get("delta").String()
					case "response.output_item.done":
						if event.Get("item.type").String() == "message" {
							id := event.Get("item.id").String()
							ended[id] = true
							if event.Get("item.content.0.text").String() != texts[id] {
								t.Fatalf("finalized text differs from its deltas: %s", event.Raw)
							}
						}
					case "response.failed", "response.completed", "response.incomplete":
						terminals++
						if event.Get("type").String() != tc.terminal {
							t.Fatalf("wrong terminal event: %s", event.Raw)
						}
					}
				}
				if terminals != 1 {
					t.Fatalf("terminal events = %d, want exactly 1", terminals)
				}
			})
		}
	}
}

func TestProviderGatewayEmptyToolSnapshotsAgree(t *testing.T) {
	gin.SetMode(gin.TestMode)
	const first = `data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_empty","function":{"name":"lookup","arguments":""}}]}}]}` + "\n\n"
	for _, translated := range []bool{false, true} {
		for _, explicitDone := range []bool{false, true} {
			for _, tc := range []struct{ name, reason, request, value, terminal string }{
				{"function stop", "stop", `{}`, "{}", "response.completed"},
				{"function tool_calls", "tool_calls", `{}`, "{}", "response.completed"},
				{"function length", "length", `{}`, "", "response.incomplete"},
				{"custom empty", "tool_calls", `{"tools":[{"type":"custom","name":"lookup"}]}`, "", "response.completed"},
			} {
				name := tc.name
				if translated {
					name = "translated/" + name
				}
				if explicitDone {
					name += "/DONE"
				}
				t.Run(name, func(t *testing.T) {
					stream := first + `data: {"choices":[{"index":0,"delta":{},"finish_reason":"` + tc.reason + `"}]}` + "\n\n"
					if explicitDone {
						stream += "data: [DONE]\n\n"
					}
					w := httptest.NewRecorder()
					c, _ := gin.CreateTestContext(w)
					c.Request = httptest.NewRequest("POST", "/v1/responses", nil)
					if translated {
						(&relayServer{}).writeProviderGatewayTranslatedChatStream(c, strings.NewReader(stream), "fixture", []byte(tc.request), []byte(tc.request), sdktranslator.FormatOpenAIResponse, "")
					} else {
						(&relayServer{}).writeProviderGatewayChatStream(c, strings.NewReader(stream), "fixture", []byte(tc.request), []byte(tc.request), false)
					}
					field, doneType := "arguments", "response.function_call_arguments.done"
					if tc.name == "custom empty" {
						field, doneType = "input", "response.custom_tool_call_input.done"
					}
					argsDone, itemDone, terminal := 0, 0, 0
					for _, event := range gatewayReviewSSEEvents(t, w.Body.String()) {
						switch event.Get("type").String() {
						case doneType:
							argsDone++
							if !event.Get(field).Exists() || event.Get(field).String() != tc.value {
								t.Fatalf("argument event differs: %s", event.Raw)
							}
						case "response.output_item.done":
							itemDone++
							if !event.Get("item."+field).Exists() || event.Get("item."+field).String() != tc.value {
								t.Fatalf("item event differs: %s", event.Raw)
							}
						case "response.completed", "response.incomplete", "response.failed":
							terminal++
							if event.Get("type").String() != tc.terminal || !event.Get("response.output.0."+field).Exists() || event.Get("response.output.0."+field).String() != tc.value {
								t.Fatalf("terminal snapshot differs: %s", event.Raw)
							}
						}
					}
					if argsDone != 1 || itemDone != 1 || terminal != 1 {
						t.Fatalf("events: args=%d item=%d terminal=%d", argsDone, itemDone, terminal)
					}
				})
			}
		}
	}
}
