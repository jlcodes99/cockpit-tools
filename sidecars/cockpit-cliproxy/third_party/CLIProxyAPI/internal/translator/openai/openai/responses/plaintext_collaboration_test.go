package responses

import (
	"context"
	"fmt"
	"testing"

	"github.com/tidwall/gjson"
)

func TestChatCollaborationCallsDeclarePlaintextInEveryResponseShape(t *testing.T) {
	for _, name := range []string{"spawn_agent", "send_message", "followup_task"} {
		t.Run(name, func(t *testing.T) {
			request := []byte(fmt.Sprintf(`{"tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":%q,"parameters":{"properties":{"message":{"type":"string","encrypted":true}}}}]}]}`, name))
			raw := []byte(fmt.Sprintf(`{"id":"chat_1","choices":[{"index":0,"message":{"role":"assistant","tool_calls":[{"id":"c1","type":"function","function":{"name":"collaboration__%s","arguments":"{\"message\":\"task\"}"}}]},"finish_reason":"tool_calls"}]}`, name))
			response := ConvertOpenAIChatCompletionsResponseToOpenAIResponsesNonStream(context.Background(), "model", request, nil, raw, nil)
			if gjson.GetBytes(response, "output.0.encrypted_function_args").Raw != "[]" {
				t.Fatalf("Chat JSON marker missing: %s", response)
			}
			chunks := []string{fmt.Sprintf(`data: {"id":"chat_1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"collaboration__%s","arguments":""}}]},"finish_reason":null}]}`, name), `data: {"id":"chat_1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"message\":\"task\"}"}}]},"finish_reason":"tool_calls"}]}`, `data: [DONE]`}
			var state any
			seen := map[string]bool{}
			for _, chunk := range chunks {
				for _, output := range ConvertOpenAIChatCompletionsResponseToOpenAIResponses(context.Background(), "model", request, nil, []byte(chunk), &state) {
					event, data := parseOpenAIResponsesSSEEvent(t, output)
					path := "item"
					switch event {
					case "response.output_item.added", "response.output_item.done":
					case "response.completed":
						path = "response.output.0"
					default:
						continue
					}
					if data.Get(path+".encrypted_function_args").Raw != "[]" {
						t.Fatalf("%s marker missing: %s", event, output)
					}
					seen[event] = true
				}
			}
			if len(seen) != 3 {
				t.Fatalf("missing shapes: %v", seen)
			}
		})
	}
}
