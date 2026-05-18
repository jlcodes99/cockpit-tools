package converters

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

func TestResponsesToGeminiRequest_PreservesFunctionItems(t *testing.T) {
	sess := &session.Session{ID: "sess_test"}
	req := &types.ResponsesRequest{
		Model: "gpt-4.1",
		Input: []interface{}{
			map[string]interface{}{
				"type":      "function_call",
				"call_id":   "weather_call",
				"name":      "get_weather",
				"arguments": `{"location":"NYC"}`,
			},
			map[string]interface{}{
				"type":    "function_call_output",
				"call_id": "weather_call",
				"output":  "Sunny, 72°F",
			},
		},
	}

	geminiReq, err := ResponsesToGeminiRequest(sess, req, "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("ResponsesToGeminiRequest failed: %v", err)
	}

	if len(geminiReq.Contents) != 2 {
		t.Fatalf("expected 2 contents, got %d", len(geminiReq.Contents))
	}

	callPart := geminiReq.Contents[0].Parts[0].FunctionCall
	if callPart == nil {
		t.Fatal("expected first content to be function call")
	}
	if callPart.Name != "get_weather" {
		t.Fatalf("expected function name get_weather, got %q", callPart.Name)
	}
	if callPart.Args["location"] != "NYC" {
		t.Fatalf("expected args.location NYC, got %#v", callPart.Args["location"])
	}

	respPart := geminiReq.Contents[1].Parts[0].FunctionResponse
	if respPart == nil {
		t.Fatal("expected second content to be function response")
	}
	if respPart.Name != "weather_call" {
		t.Fatalf("expected function response name weather_call, got %q", respPart.Name)
	}
	if respPart.Response["result"] != "Sunny, 72°F" {
		t.Fatalf("expected tool result preserved, got %#v", respPart.Response["result"])
	}
}

func TestGeminiResponseToResponses_UsesStableFunctionNameAsCallID(t *testing.T) {
	geminiResp := map[string]interface{}{
		"candidates": []interface{}{
			map[string]interface{}{
				"content": map[string]interface{}{
					"parts": []interface{}{
						map[string]interface{}{
							"functionCall": map[string]interface{}{
								"name": "get_weather",
								"args": map[string]interface{}{"location": "NYC"},
							},
						},
					},
				},
				"finishReason": "STOP",
			},
		},
	}

	resp, err := GeminiResponseToResponses(geminiResp, "sess_test")
	if err != nil {
		t.Fatalf("GeminiResponseToResponses failed: %v", err)
	}
	if len(resp.Output) != 1 {
		t.Fatalf("expected 1 output item, got %d", len(resp.Output))
	}

	if resp.Output[0].Name != "get_weather" {
		t.Fatalf("expected name get_weather, got %#v", resp.Output[0].Name)
	}
	if resp.Output[0].CallID != "get_weather" {
		t.Fatalf("expected call_id get_weather, got %#v", resp.Output[0].CallID)
	}

	followupReq := &types.ResponsesRequest{
		Model: "gpt-4.1",
		Input: []interface{}{
			map[string]interface{}{
				"type":    "function_call_output",
				"call_id": resp.Output[0].CallID,
				"output":  "Sunny, 72°F",
			},
		},
	}
	geminiReq, err := ResponsesToGeminiRequest(&session.Session{ID: "sess_test"}, followupReq, "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("ResponsesToGeminiRequest followup failed: %v", err)
	}
	if len(geminiReq.Contents) != 1 || len(geminiReq.Contents[0].Parts) != 1 {
		t.Fatalf("expected single function response content, got %#v", geminiReq.Contents)
	}
	functionResponse := geminiReq.Contents[0].Parts[0].FunctionResponse
	if functionResponse == nil {
		t.Fatal("expected function response in followup request")
	}
	if functionResponse.Name != "get_weather" {
		t.Fatalf("expected function response name get_weather, got %q", functionResponse.Name)
	}
}

func TestResponsesToGeminiRequest_PreservesStructuredFunctionOutput(t *testing.T) {
	sess := &session.Session{ID: "sess_test"}
	req := &types.ResponsesRequest{
		Model: "gpt-4.1",
		Input: []interface{}{
			map[string]interface{}{
				"type":    "function_call_output",
				"call_id": "weather_call",
				"output":  map[string]interface{}{"temperature": 72, "condition": "sunny"},
			},
		},
	}

	geminiReq, err := ResponsesToGeminiRequest(sess, req, "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("ResponsesToGeminiRequest failed: %v", err)
	}
	if len(geminiReq.Contents) != 1 || len(geminiReq.Contents[0].Parts) != 1 {
		t.Fatalf("expected single function response content, got %#v", geminiReq.Contents)
	}
	functionResponse := geminiReq.Contents[0].Parts[0].FunctionResponse
	if functionResponse == nil {
		t.Fatal("expected function response")
	}
	if functionResponse.Name != "weather_call" {
		t.Fatalf("expected function response name weather_call, got %q", functionResponse.Name)
	}
	if functionResponse.Response["temperature"] != 72 {
		t.Fatalf("expected temperature 72, got %#v", functionResponse.Response["temperature"])
	}
	if functionResponse.Response["condition"] != "sunny" {
		t.Fatalf("expected condition sunny, got %#v", functionResponse.Response["condition"])
	}
}

func TestConvertGeminiStreamToResponses_MapsFinishReasonToStatus(t *testing.T) {
	ctx := context.Background()
	cases := []struct {
		name         string
		finishReason string
		expectStatus string
	}{
		{name: "stop maps to completed", finishReason: "STOP", expectStatus: "completed"},
		{name: "max tokens maps to incomplete", finishReason: "MAX_TOKENS", expectStatus: "incomplete"},
		{name: "safety maps to failed", finishReason: "SAFETY", expectStatus: "failed"},
	}

	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			var state any
			line := `data: {"candidates":[{"content":{"parts":[{"text":"hello"}]},"finishReason":"` + tt.finishReason + `"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5}}`
			events := ConvertGeminiStreamToResponses(ctx, "gemini-2.5-pro", []byte(`{"model":"gpt-4.1"}`), nil, []byte(line), &state)

			var completed string
			for _, ev := range events {
				if strings.Contains(ev, "response.completed") {
					completed = ev
					break
				}
			}
			if completed == "" {
				t.Fatalf("expected response.completed event, got %#v", events)
			}

			var payload struct {
				Response struct {
					Status string `json:"status"`
				} `json:"response"`
			}
			jsonLine := completed
			if strings.HasPrefix(jsonLine, "event: ") {
				parts := strings.Split(completed, "\n")
				for _, part := range parts {
					if strings.HasPrefix(part, "data: ") {
						jsonLine = strings.TrimPrefix(part, "data: ")
						break
					}
				}
			}
			if err := json.Unmarshal([]byte(jsonLine), &payload); err != nil {
				t.Fatalf("unmarshal completed event failed: %v; event=%s", err, completed)
			}
			if payload.Response.Status != tt.expectStatus {
				t.Fatalf("expected status %q, got %q", tt.expectStatus, payload.Response.Status)
			}
		})
	}
}

func TestConvertGeminiStreamToResponses_ThoughtToReasoning(t *testing.T) {
	ctx := context.Background()
	var state any
	originalReq := []byte(`{"model":"deepseek-v4-pro"}`)

	first := `data: {"candidates":[{"content":{"role":"model","parts":[{"text":"gemini thinking","thought":true},{"text":"answer"}]}}]}`
	second := `data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5}}`

	events := ConvertGeminiStreamToResponses(ctx, "deepseek-v4-pro", originalReq, nil, []byte(first), &state)
	events = append(events, ConvertGeminiStreamToResponses(ctx, "deepseek-v4-pro", originalReq, nil, []byte(second), &state)...)
	joined := strings.Join(events, "\n")

	for _, want := range []string{
		`response.reasoning_summary_text.delta`,
		`"text":"gemini thinking"`,
		`"type":"reasoning"`,
		`"delta":"answer"`,
		`"type":"response.completed"`,
	} {
		if !strings.Contains(joined, want) {
			t.Fatalf("expected %q in converted events, got %v", want, events)
		}
	}
}

func TestConvertGeminiStreamToResponses_PreservesFunctionCallCallID(t *testing.T) {
	ctx := context.Background()
	var state any
	line := `data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"get_weather","args":{"location":"NYC"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5}}`
	events := ConvertGeminiStreamToResponses(ctx, "gemini-2.5-pro", []byte(`{"model":"gpt-4.1"}`), nil, []byte(line), &state)

	var completed string
	for _, ev := range events {
		if strings.Contains(ev, "response.completed") {
			completed = ev
			break
		}
	}
	if completed == "" {
		t.Fatalf("expected response.completed event, got %#v", events)
	}

	jsonLine := completed
	if strings.HasPrefix(jsonLine, "event: ") {
		parts := strings.Split(completed, "\n")
		for _, part := range parts {
			if strings.HasPrefix(part, "data: ") {
				jsonLine = strings.TrimPrefix(part, "data: ")
				break
			}
		}
	}

	var payload map[string]interface{}
	if err := json.Unmarshal([]byte(jsonLine), &payload); err != nil {
		t.Fatalf("unmarshal completed event failed: %v", err)
	}
	response := payload["response"].(map[string]interface{})
	output := response["output"].([]interface{})
	if len(output) != 1 {
		t.Fatalf("expected 1 output item, got %d", len(output))
	}
	item := output[0].(map[string]interface{})
	content := item["content"].(map[string]interface{})
	if content["name"] != "get_weather" {
		t.Fatalf("expected function name get_weather, got %#v", content["name"])
	}
	if content["call_id"] != "get_weather" {
		t.Fatalf("expected call_id get_weather, got %#v", content["call_id"])
	}
}

func TestConvertGeminiStreamToResponses_PureToolCallWithoutText(t *testing.T) {
	ctx := context.Background()
	var state any
	line := `data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"search_docs","args":{"query":"responses api"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":3}}`
	events := ConvertGeminiStreamToResponses(ctx, "gemini-2.5-pro", []byte(`{"model":"gpt-4.1"}`), nil, []byte(line), &state)

	var completed string
	for _, ev := range events {
		if strings.Contains(ev, "response.completed") {
			completed = ev
			break
		}
	}
	if completed == "" {
		t.Fatalf("expected response.completed event, got %#v", events)
	}
	if strings.Contains(completed, `"type":"message"`) {
		t.Fatalf("expected pure tool call completed event without message item, got %s", completed)
	}
	if !strings.Contains(completed, `"type":"function_call"`) {
		t.Fatalf("expected pure tool call completed event to contain function_call, got %s", completed)
	}
}

func TestConvertGeminiStreamToResponses_MultipleFunctionCalls(t *testing.T) {
	ctx := context.Background()
	var state any
	line := `data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"get_weather","args":{"location":"NYC"}}},{"functionCall":{"name":"get_time","args":{"timezone":"UTC"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":18,"candidatesTokenCount":6}}`
	events := ConvertGeminiStreamToResponses(ctx, "gemini-2.5-pro", []byte(`{"model":"gpt-4.1"}`), nil, []byte(line), &state)

	var completed string
	for _, ev := range events {
		if strings.Contains(ev, "response.completed") {
			completed = ev
			break
		}
	}
	if completed == "" {
		t.Fatalf("expected response.completed event, got %#v", events)
	}
	if strings.Count(completed, `"type":"function_call"`) != 2 {
		t.Fatalf("expected two function_call items, got %s", completed)
	}
	if !strings.Contains(completed, `"call_id":"get_weather"`) || !strings.Contains(completed, `"call_id":"get_time"`) {
		t.Fatalf("expected both function call ids in completed event, got %s", completed)
	}
}

func TestConvertGeminiStreamToResponses_UsesLateUsageMetadata(t *testing.T) {
	ctx := context.Background()
	var state any
	originalReq := []byte(`{"model":"gpt-4.1"}`)

	first := `data: {"candidates":[{"content":{"parts":[{"text":"hello"}]}}]}`
	second := `data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":7,"cachedContentTokenCount":5}}`

	_ = ConvertGeminiStreamToResponses(ctx, "gemini-2.5-pro", originalReq, nil, []byte(first), &state)
	events := ConvertGeminiStreamToResponses(ctx, "gemini-2.5-pro", originalReq, nil, []byte(second), &state)

	var completed string
	for _, ev := range events {
		if strings.Contains(ev, "response.completed") {
			completed = ev
			break
		}
	}
	if completed == "" {
		t.Fatalf("expected response.completed event, got %#v", events)
	}

	var payload map[string]interface{}
	jsonLine := completed
	if strings.HasPrefix(jsonLine, "event: ") {
		parts := strings.Split(completed, "\n")
		for _, part := range parts {
			if strings.HasPrefix(part, "data: ") {
				jsonLine = strings.TrimPrefix(part, "data: ")
				break
			}
		}
	}
	if err := json.Unmarshal([]byte(jsonLine), &payload); err != nil {
		t.Fatalf("unmarshal completed event failed: %v", err)
	}
	usage := payload["response"].(map[string]interface{})["usage"].(map[string]interface{})
	if usage["input_tokens"].(float64) != 15 {
		t.Fatalf("expected input_tokens 15 after cached deduction, got %#v", usage["input_tokens"])
	}
	if usage["output_tokens"].(float64) != 7 {
		t.Fatalf("expected output_tokens 7, got %#v", usage["output_tokens"])
	}
}

func TestResponsesToGeminiRequest_FunctionCallFallsBackToNestedContent(t *testing.T) {
	sess := &session.Session{ID: "sess_test"}
	req := &types.ResponsesRequest{
		Model: "gpt-4.1",
		Input: []interface{}{
			map[string]interface{}{
				"type": "function_call",
				"content": map[string]interface{}{
					"name":      "get_weather",
					"arguments": `{"location":"Tokyo"}`,
				},
			},
		},
	}

	geminiReq, err := ResponsesToGeminiRequest(sess, req, "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("ResponsesToGeminiRequest failed: %v", err)
	}
	functionCall := geminiReq.Contents[0].Parts[0].FunctionCall
	if functionCall == nil {
		t.Fatal("expected function call")
	}
	if functionCall.Name != "get_weather" {
		t.Fatalf("expected function name get_weather, got %q", functionCall.Name)
	}
	if functionCall.Args["location"] != "Tokyo" {
		t.Fatalf("expected args.location Tokyo, got %#v", functionCall.Args["location"])
	}
}

func TestResponsesResponseToGemini_PreservesStructuredFunctionOutput(t *testing.T) {
	resp, err := ResponsesResponseToGemini(map[string]interface{}{
		"status": "completed",
		"output": []interface{}{
			map[string]interface{}{
				"type":    "function_call_output",
				"call_id": "weather_call",
				"output": map[string]interface{}{
					"temperature": 72,
					"condition":   "sunny",
				},
			},
		},
	})
	if err != nil {
		t.Fatalf("ResponsesResponseToGemini failed: %v", err)
	}
	if len(resp.Candidates) != 1 || resp.Candidates[0].Content == nil || len(resp.Candidates[0].Content.Parts) != 1 {
		t.Fatalf("expected single function response part, got %#v", resp.Candidates)
	}
	functionResponse := resp.Candidates[0].Content.Parts[0].FunctionResponse
	if functionResponse == nil {
		t.Fatal("expected function response")
	}
	if functionResponse.Name != "weather_call" {
		t.Fatalf("expected function response name weather_call, got %q", functionResponse.Name)
	}
	if functionResponse.Response["temperature"] != 72 {
		t.Fatalf("expected temperature 72, got %#v", functionResponse.Response["temperature"])
	}
	if functionResponse.Response["condition"] != "sunny" {
		t.Fatalf("expected condition sunny, got %#v", functionResponse.Response["condition"])
	}
}

func TestResponsesResponseToGemini_WrapsScalarFunctionOutputConsistently(t *testing.T) {
	resp, err := ResponsesResponseToGemini(map[string]interface{}{
		"status": "completed",
		"output": []interface{}{
			map[string]interface{}{
				"type":    "function_call_output",
				"call_id": "weather_call",
				"output":  true,
			},
		},
	})
	if err != nil {
		t.Fatalf("ResponsesResponseToGemini failed: %v", err)
	}
	functionResponse := resp.Candidates[0].Content.Parts[0].FunctionResponse
	if functionResponse == nil {
		t.Fatal("expected function response")
	}
	if functionResponse.Response["result"] != true {
		t.Fatalf("expected scalar output wrapped under result, got %#v", functionResponse.Response)
	}
}

func TestResponsesResponseToGemini_ExtractsMessageTextBlocks(t *testing.T) {
	resp, err := ResponsesResponseToGemini(map[string]interface{}{
		"status": "completed",
		"output": []interface{}{
			map[string]interface{}{
				"type": "message",
				"content": []interface{}{
					map[string]interface{}{"type": "output_text", "text": "hello"},
					map[string]interface{}{"type": "text", "text": "world"},
				},
			},
		},
	})
	if err != nil {
		t.Fatalf("ResponsesResponseToGemini failed: %v", err)
	}
	parts := resp.Candidates[0].Content.Parts
	if len(parts) != 2 {
		t.Fatalf("expected 2 text parts, got %#v", parts)
	}
	if parts[0].Text != "hello" || parts[1].Text != "world" {
		t.Fatalf("unexpected text parts: %#v", parts)
	}
}

func TestResponsesResponseToGemini_UsesFunctionCallArguments(t *testing.T) {
	resp, err := ResponsesResponseToGemini(map[string]interface{}{
		"status": "completed",
		"output": []interface{}{
			map[string]interface{}{
				"type":      "function_call",
				"name":      "get_weather",
				"arguments": `{"location":"Paris"}`,
			},
		},
	})
	if err != nil {
		t.Fatalf("ResponsesResponseToGemini failed: %v", err)
	}
	functionCall := resp.Candidates[0].Content.Parts[0].FunctionCall
	if functionCall == nil {
		t.Fatal("expected function call")
	}
	if functionCall.Name != "get_weather" {
		t.Fatalf("expected function name get_weather, got %q", functionCall.Name)
	}
	if functionCall.Args["location"] != "Paris" {
		t.Fatalf("expected args.location Paris, got %#v", functionCall.Args["location"])
	}
}

func TestResponsesResponseToGemini_MapsStatusToFinishReason(t *testing.T) {
	cases := []struct {
		status       string
		expectReason string
	}{
		{status: "completed", expectReason: "STOP"},
		{status: "incomplete", expectReason: "MAX_TOKENS"},
		{status: "failed", expectReason: "SAFETY"},
	}

	for _, tt := range cases {
		t.Run(tt.status, func(t *testing.T) {
			resp, err := ResponsesResponseToGemini(map[string]interface{}{
				"status": tt.status,
				"output": []interface{}{},
			})
			if err != nil {
				t.Fatalf("ResponsesResponseToGemini failed: %v", err)
			}
			if len(resp.Candidates) != 1 {
				t.Fatalf("expected 1 candidate, got %d", len(resp.Candidates))
			}
			if resp.Candidates[0].FinishReason != tt.expectReason {
				t.Fatalf("expected finish reason %q, got %q", tt.expectReason, resp.Candidates[0].FinishReason)
			}
		})
	}
}

func TestResponsesToGeminiRequest_InvalidFunctionArgumentsFallsBackToEmptyArgs(t *testing.T) {
	sess := &session.Session{ID: "sess_test"}
	req := &types.ResponsesRequest{
		Model: "gpt-4.1",
		Input: []interface{}{
			map[string]interface{}{
				"type":      "function_call",
				"call_id":   "call_invalid",
				"name":      "get_weather",
				"arguments": `{bad json}`,
			},
		},
	}

	geminiReq, err := ResponsesToGeminiRequest(sess, req, "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("ResponsesToGeminiRequest failed: %v", err)
	}
	functionCall := geminiReq.Contents[0].Parts[0].FunctionCall
	if functionCall == nil {
		t.Fatal("expected function call")
	}
	if functionCall.Name != "get_weather" {
		t.Fatalf("expected function name get_weather, got %q", functionCall.Name)
	}
	if functionCall.Args != nil {
		t.Fatalf("expected invalid arguments to fall back to nil args, got %#v", functionCall.Args)
	}
}

func TestResponsesToGeminiRequest_FunctionCallOutputMissingCallIDSkipsItem(t *testing.T) {
	sess := &session.Session{ID: "sess_test"}
	req := &types.ResponsesRequest{
		Model: "gpt-4.1",
		Input: []interface{}{
			map[string]interface{}{
				"type":   "function_call_output",
				"output": "Sunny",
			},
		},
	}

	geminiReq, err := ResponsesToGeminiRequest(sess, req, "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("ResponsesToGeminiRequest failed: %v", err)
	}
	if len(geminiReq.Contents) != 0 {
		t.Fatalf("expected missing call_id item to be skipped, got %#v", geminiReq.Contents)
	}
}

func TestResponsesToGeminiRequest_FunctionCallMissingNameSkipsItem(t *testing.T) {
	sess := &session.Session{ID: "sess_test"}
	req := &types.ResponsesRequest{
		Model: "gpt-4.1",
		Input: []interface{}{
			map[string]interface{}{
				"type":      "function_call",
				"call_id":   "call_1",
				"arguments": `{"location":"Tokyo"}`,
			},
		},
	}

	geminiReq, err := ResponsesToGeminiRequest(sess, req, "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("ResponsesToGeminiRequest failed: %v", err)
	}
	if len(geminiReq.Contents) != 0 {
		t.Fatalf("expected missing name item to be skipped, got %#v", geminiReq.Contents)
	}
}

func TestResponsesToGeminiRequest_FunctionCallWithJSONArrayArgumentsFallsBackToNilArgs(t *testing.T) {
	sess := &session.Session{ID: "sess_test"}
	req := &types.ResponsesRequest{
		Model: "gpt-4.1",
		Input: []interface{}{
			map[string]interface{}{
				"type":      "function_call",
				"call_id":   "call_array",
				"name":      "get_weather",
				"arguments": `["Tokyo","Berlin"]`,
			},
		},
	}

	geminiReq, err := ResponsesToGeminiRequest(sess, req, "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("ResponsesToGeminiRequest failed: %v", err)
	}
	functionCall := geminiReq.Contents[0].Parts[0].FunctionCall
	if functionCall == nil {
		t.Fatal("expected function call")
	}
	if functionCall.Args != nil {
		t.Fatalf("expected JSON array arguments to fall back to nil args, got %#v", functionCall.Args)
	}
}

func TestGeminiResponsesRoundtrip_FunctionResponseObjectShape(t *testing.T) {
	originalReq := &types.GeminiRequest{
		Contents: []types.GeminiContent{
			{
				Role: "user",
				Parts: []types.GeminiPart{
					{
						FunctionResponse: &types.GeminiFunctionResponse{
							Name: "weather_call",
							Response: map[string]interface{}{
								"temperature": 72,
								"condition":   "sunny",
							},
						},
					},
				},
			},
		},
	}

	responsesReq, err := GeminiToResponsesRequest(originalReq, "gpt-4.1")
	if err != nil {
		t.Fatalf("GeminiToResponsesRequest failed: %v", err)
	}
	input, ok := responsesReq["input"].([]types.ResponsesItem)
	if !ok || len(input) != 1 {
		t.Fatalf("expected single responses item, got %#v", responsesReq["input"])
	}
	if input[0].Type != "function_call_output" || input[0].CallID != "weather_call" {
		t.Fatalf("unexpected responses item: %#v", input[0])
	}

	responsesResp := map[string]interface{}{
		"status": "completed",
		"output": []interface{}{
			map[string]interface{}{
				"type":    input[0].Type,
				"call_id": input[0].CallID,
				"output":  input[0].Output,
			},
		},
	}
	geminiResp, err := ResponsesResponseToGemini(responsesResp)
	if err != nil {
		t.Fatalf("ResponsesResponseToGemini failed: %v", err)
	}
	functionResponse := geminiResp.Candidates[0].Content.Parts[0].FunctionResponse
	if functionResponse == nil {
		t.Fatal("expected function response")
	}
	if functionResponse.Name != "weather_call" {
		t.Fatalf("expected function response name weather_call, got %q", functionResponse.Name)
	}
	if functionResponse.Response["temperature"] != 72 || functionResponse.Response["condition"] != "sunny" {
		t.Fatalf("unexpected function response payload: %#v", functionResponse.Response)
	}
}

func TestGeminiResponsesRoundtrip_FunctionCallPreservesArguments(t *testing.T) {
	originalResp := map[string]interface{}{
		"candidates": []interface{}{
			map[string]interface{}{
				"content": map[string]interface{}{
					"parts": []interface{}{
						map[string]interface{}{
							"functionCall": map[string]interface{}{
								"name": "get_weather",
								"args": map[string]interface{}{"location": "Paris", "unit": "celsius"},
							},
						},
					},
				},
				"finishReason": "STOP",
			},
		},
	}

	responsesResp, err := GeminiResponseToResponses(originalResp, "sess_test")
	if err != nil {
		t.Fatalf("GeminiResponseToResponses failed: %v", err)
	}
	if len(responsesResp.Output) != 1 {
		t.Fatalf("expected 1 output item, got %d", len(responsesResp.Output))
	}
	if responsesResp.Output[0].Type != "function_call" {
		t.Fatalf("expected function_call, got %#v", responsesResp.Output[0])
	}

	convertedGemini, err := ResponsesResponseToGemini(map[string]interface{}{
		"status": "completed",
		"output": []interface{}{
			map[string]interface{}{
				"type":      responsesResp.Output[0].Type,
				"call_id":   responsesResp.Output[0].CallID,
				"name":      responsesResp.Output[0].Name,
				"arguments": responsesResp.Output[0].Arguments,
			},
		},
	})
	if err != nil {
		t.Fatalf("ResponsesResponseToGemini failed: %v", err)
	}
	functionCall := convertedGemini.Candidates[0].Content.Parts[0].FunctionCall
	if functionCall == nil {
		t.Fatal("expected function call")
	}
	if functionCall.Name != "get_weather" {
		t.Fatalf("expected function name get_weather, got %q", functionCall.Name)
	}
	if functionCall.Args["location"] != "Paris" || functionCall.Args["unit"] != "celsius" {
		t.Fatalf("unexpected function args: %#v", functionCall.Args)
	}
}

func TestResponsesResponseToGemini_MessageContentStringIsIgnoredSafely(t *testing.T) {
	resp, err := ResponsesResponseToGemini(map[string]interface{}{
		"status": "completed",
		"output": []interface{}{
			map[string]interface{}{
				"type":    "message",
				"content": "plain string",
			},
		},
	})
	if err != nil {
		t.Fatalf("ResponsesResponseToGemini failed: %v", err)
	}
	if len(resp.Candidates) != 1 {
		t.Fatalf("expected 1 candidate, got %d", len(resp.Candidates))
	}
	if resp.Candidates[0].Content == nil {
		t.Fatal("expected content")
	}
	if len(resp.Candidates[0].Content.Parts) != 0 {
		t.Fatalf("expected unsupported string content to be ignored safely, got %#v", resp.Candidates[0].Content.Parts)
	}
}

func TestResponsesToGeminiRequest_FunctionCallOutputFallsBackToName(t *testing.T) {
	sess := &session.Session{ID: "sess_test"}
	req := &types.ResponsesRequest{
		Model: "gpt-4.1",
		Input: []interface{}{
			map[string]interface{}{
				"type":   "function_call_output",
				"name":   "weather_call",
				"output": "Sunny, 72°F",
			},
		},
	}

	geminiReq, err := ResponsesToGeminiRequest(sess, req, "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("ResponsesToGeminiRequest failed: %v", err)
	}
	functionResponse := geminiReq.Contents[0].Parts[0].FunctionResponse
	if functionResponse == nil {
		t.Fatal("expected function response")
	}
	if functionResponse.Name != "weather_call" {
		t.Fatalf("expected function response name weather_call, got %q", functionResponse.Name)
	}
}
