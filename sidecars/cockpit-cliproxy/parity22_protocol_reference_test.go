package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"strings"
	"testing"

	responsesconverter "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/openai/openai/responses"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	_ "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator/builtin"
)

// Regenerate with COCKPIT_PARITY22_PROTOCOL_FIXTURE=../../src-tauri/src/modules/codex_native_api/fixtures/parity22_protocol_go.json go test -run '^TestParity22ProtocolReference$' .
// Production Go sources are deliberately not changed by this reference harness.
func TestParity22ProtocolReference(t *testing.T) {
	cases := []map[string]any{}
	decode := func(raw []byte) map[string]any {
		var value map[string]any
		if err := json.Unmarshal(raw, &value); err != nil {
			t.Fatal(err)
		}
		return value
	}
	addRequest := func(name, source, input string, fields ...string) {
		format := sdktranslator.FormatClaude
		if source == "gemini" {
			format = sdktranslator.FormatGemini
		}
		if source == "chat" {
			format = sdktranslator.FormatOpenAI
		}
		wire := decode(sdktranslator.TranslateRequest(format, sdktranslator.FormatCodex, "parity-model", []byte(input), false))
		expected := map[string]any{}
		for _, field := range fields {
			expected[field] = wire[field]
		}
		cases = append(cases, map[string]any{"name": name, "kind": "canonical", "source": source, "input": decode([]byte(input)), "expected": expected})
	}
	for _, budget := range []int{-1, 0, 1, 512, 513, 1024, 1025, 1500, 2048, 2049, 8192, 8193, 24576, 24577, 32768} {
		addRequest(fmt.Sprintf("body_budget_claude_%d", budget), "claude", fmt.Sprintf(`{"model":"parity-model","messages":[],"thinking":{"type":"enabled","budget_tokens":%d}}`, budget), "reasoning")
		addRequest(fmt.Sprintf("body_budget_gemini_%d", budget), "gemini", fmt.Sprintf(`{"contents":[],"generationConfig":{"thinkingConfig":{"thinkingBudget":%d}}}`, budget), "reasoning")
	}
	for _, mode := range []string{"adaptive", "auto", "disabled"} {
		addRequest("claude_mode_"+mode, "claude", fmt.Sprintf(`{"model":"parity-model","messages":[],"thinking":{"type":%q}}`, mode), "reasoning")
	}
	for _, display := range []string{"omitted", "summarized"} {
		addRequest("claude_summary_"+display, "claude", fmt.Sprintf(`{"model":"parity-model","messages":[],"thinking":{"type":"enabled","budget_tokens":4096,"display":%q}}`, display), "reasoning")
	}
	for _, field := range []string{"includeThoughts", "include_thoughts"} {
		for _, enabled := range []bool{false, true} {
			addRequest(fmt.Sprintf("gemini_summary_%s_%t", field, enabled), "gemini", fmt.Sprintf(`{"contents":[],"generationConfig":{"thinkingConfig":{"thinkingBudget":-1,%q:%t}}}`, field, enabled), "reasoning")
		}
	}
	for _, extra := range []string{`"reasoning_effort":"high"`, `"reasoning_effort":"none"`, `"reasoning_effort":"high","reasoning":{"exclude":true}`, `"reasoning_effort":"high","extra_body":{"google":{"thinking_config":{"include_thoughts":false}}}`, `"reasoning":{"generate_summary":"detailed"}`} {
		addRequest(fmt.Sprintf("chat_summary_%d", len(cases)), "chat", `{"model":"parity-model","messages":[],`+extra+`}`, "reasoning")
	}
	for _, schema := range []string{
		`{"type":"json_schema","name":"custom_schema","strict":false,"schema":{"type":"object","properties":{"a":{"type":"string"}}}}`,
		`{"type":"json_schema","strict":true,"schema":{"type":"object","properties":{"a":{"type":"string"}}}}`,
		`{"type":"json_schema","name":"fully_required","schema":{"type":"object","properties":{"a":{"type":"string"}},"required":["a"]}}`,
		`{"type":"json_schema","schema":{"type":"object","properties":{"a":{"type":"array","items":{"type":"object","properties":{"x":{"type":"string"}}}}},"required":["a"]}}`,
	} {
		addRequest(fmt.Sprintf("claude_schema_%d", len(cases)), "claude", `{"model":"parity-model","messages":[],"output_config":{"format":`+schema+`}}`, "text")
	}
	for _, tier := range []string{"priority", "fast", "flex", "default", " PRIORITY "} {
		addRequest("claude_tier_"+tier, "claude", fmt.Sprintf(`{"model":"parity-model","messages":[],"service_tier":%q}`, tier), "service_tier")
		addRequest("gemini_tier_"+tier, "gemini", fmt.Sprintf(`{"contents":[],"service_tier":%q}`, tier), "service_tier")
	}
	addRequest("claude_speed_fast", "claude", `{"model":"parity-model","messages":[],"speed":"fast"}`, "service_tier")
	addRequest("claude_system_reminders", "claude", `{"model":"parity-model","messages":[{"role":"user","content":"hi"},{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"f","input":{}}]},{"role":"system","content":"project instructions"},{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"ok"},{"type":"text","text":"next"}]}]}`, "input")
	addRequest("gemini_call_id_reverse", "gemini", `{"contents":[{"role":"model","parts":[{"functionCall":{"call_id":"a","name":"f","args":{}}},{"functionCall":{"call_id":"b","name":"f","args":{}}}]},{"role":"user","parts":[{"functionResponse":{"call_id":"b","name":"f","response":{"b":2}}},{"functionResponse":{"call_id":"a","name":"f","response":{"a":1}}}]}]}`, "input")
	for _, arguments := range []string{`{"a":2}`, `"{\"a\":2}"`, `null`} {
		input := fmt.Sprintf(`{"model":"parity-model","messages":[{"role":"assistant","content":"","tool_calls":[{"id":"c","function":{"name":"f","arguments":%s}}]},{"role":"tool","tool_call_id":"c","content":"ok"}]}`, arguments)
		chat, _, err := buildOpenAIChatRequestFromOllama([]byte(input))
		if err != nil {
			t.Fatal(err)
		}
		wire := decode(chat)
		calls := wire["messages"].([]any)[0].(map[string]any)["tool_calls"]
		cases = append(cases, map[string]any{"name": "ollama_arguments_" + arguments, "kind": "ollama_arguments", "input": decode([]byte(input)), "expected": calls})
	}
	requests := []string{
		`{"model":"parity-model","input":[{"role":"user","content":"hi"}],"tools":[{"type":"namespace","name":"terminal","tools":[{"type":"function","name":"exec","parameters":{"type":"object"}}]}]}`,
		`{"model":"parity-model","input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"terminal","tools":[{"type":"custom","name":"patch"}]}]}]}`,
		`{"model":"parity-model","input":[],"tools":[{"type":"function","name":"terminal__patch","parameters":{"type":"object"}}],"input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"terminal","tools":[{"type":"custom","name":"patch"}]}]}]}`,
		`{"model":"parity-model","input":[{"type":"custom_tool_call","namespace":"terminal","name":"patch","call_id":"a","input":"*** patch\n猫"},{"type":"message","role":"user","content":"update"},{"type":"custom_tool_call_output","call_id":"a","output":"ok"}],"tools":[{"type":"namespace","name":"terminal","tools":[{"type":"custom","name":"patch"}]}]}`,
		`{"model":"parity-model","input":[{"type":"reasoning","summary":[{"type":"summary_text","text":"plan"}]},{"role":"assistant","content":"call it"},{"type":"function_call","name":"f","call_id":"a","arguments":"{}"},{"role":"user","content":"update"},{"type":"function_call_output","call_id":"a","output":"ok"}],"tools":[{"type":"function","name":"f","parameters":{}}]}`,
		`{"model":"parity-model","input":[],"tools":[{"type":"namespace","name":"terminal","tools":[{"type":"custom","name":"patch"}]}],"tool_choice":{"type":"custom","namespace":"terminal","name":"patch"}}`,
	}
	for index, input := range requests {
		wire := decode(responsesconverter.ConvertOpenAIResponsesRequestToOpenAIChatCompletions("parity-model", []byte(input), false))
		expected := map[string]any{"messages": wire["messages"], "tools": wire["tools"]}
		cases = append(cases, map[string]any{"name": fmt.Sprintf("responses_chat_%d", index), "kind": "responses_chat", "input": decode([]byte(input)), "expected": expected, "go_tool_choice": wire["tool_choice"]})
	}
	outputs := []struct{ name, request, response string }{
		{"reasoning_alias", `{"reasoning":{"effort":"high"}}`, `{"id":"r","created":1,"model":"parity-model","choices":[{"index":0,"message":{"role":"assistant","reasoning":"plan","content":"answer"},"finish_reason":"stop"}]}`},
		{"reasoning_empty_fallback", `{"reasoning":{"effort":"high"}}`, `{"id":"r","created":1,"model":"parity-model","choices":[{"index":0,"message":{"role":"assistant","reasoning_content":"","reasoning":"plan","content":"answer"},"finish_reason":"stop"}]}`},
		{"namespace_custom", `{"tools":[{"type":"namespace","name":"terminal","tools":[{"type":"custom","name":"patch"}]}]}`, `{"id":"r","created":1,"model":"parity-model","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-a","type":"function","function":{"name":"terminal__patch","arguments":"{\"input\":\"*** patch\\n猫\\\"\\\\😀\"}"}}]},"finish_reason":"tool_calls"}]}`},
		{"namespace_function", `{"tools":[{"type":"namespace","name":"terminal","tools":[{"type":"function","name":"exec","parameters":{}}]}]}`, `{"id":"r","created":1,"model":"parity-model","choices":[{"index":0,"message":{"role":"assistant","tool_calls":[{"id":"call-a","type":"function","function":{"name":"terminal__exec","arguments":"{\"x\":1}"}}]},"finish_reason":"tool_calls"}]}`},
	}
	late := outputs[2]
	late.name = "namespace_custom_late_name"
	outputs = append(outputs, late)
	for _, item := range outputs {
		raw := responsesconverter.ConvertOpenAIChatCompletionsResponseToOpenAIResponsesNonStream(context.Background(), "parity-model", []byte(item.request), nil, []byte(item.response), nil)
		input := decode([]byte(item.response))
		choice := input["choices"].([]any)[0].(map[string]any)
		message := choice["message"].(map[string]any)
		delta := map[string]any{}
		for key, value := range message {
			if key != "tool_calls" {
				delta[key] = value
			}
		}
		chunks := []any{map[string]any{"id": "streamed", "choices": []any{map[string]any{"index": 0, "delta": delta, "finish_reason": nil}}}}
		if calls, ok := message["tool_calls"].([]any); ok {
			for index, rawCall := range calls {
				call := rawCall.(map[string]any)
				function := call["function"].(map[string]any)
				name := function["name"].(string)
				nameFragments := []string{name}
				if item.name == "namespace_custom_late_name" {
					nameFragments = []string{""}
				}
				for _, fragment := range nameFragments {
					chunks = append(chunks, map[string]any{"choices": []any{map[string]any{"index": 0, "delta": map[string]any{"tool_calls": []any{map[string]any{"index": index, "id": call["id"], "type": "function", "function": map[string]any{"name": fragment, "arguments": ""}}}}, "finish_reason": nil}}})
				}
				for _, fragment := range function["arguments"].(string) {
					chunks = append(chunks, map[string]any{"choices": []any{map[string]any{"index": 0, "delta": map[string]any{"tool_calls": []any{map[string]any{"index": index, "function": map[string]any{"arguments": string(fragment)}}}}, "finish_reason": nil}}})
				}
				if item.name == "namespace_custom_late_name" {
					chunks = append(chunks, map[string]any{"choices": []any{map[string]any{"index": 0, "delta": map[string]any{"tool_calls": []any{map[string]any{"index": index, "id": call["id"], "type": "function", "function": map[string]any{"name": name, "arguments": ""}}}}, "finish_reason": nil}}})
				}

			}
		}
		chunks = append(chunks, map[string]any{"choices": []any{map[string]any{"index": 0, "delta": map[string]any{}, "finish_reason": choice["finish_reason"]}}})
		var streamState any
		var streamed any
		for _, chunk := range append(chunks, "[DONE]") {
			payload := []byte("[DONE]")
			if chunk != "[DONE]" {
				payload, _ = json.Marshal(chunk)
			}
			for _, event := range responsesconverter.ConvertOpenAIChatCompletionsResponseToOpenAIResponses(context.Background(), "parity-model", []byte(item.request), nil, payload, &streamState) {
				for _, line := range strings.Split(string(event), "\n") {
					if strings.HasPrefix(line, "data:") {
						value := decode([]byte(strings.TrimSpace(strings.TrimPrefix(line, "data:"))))
						if value["type"] == "response.completed" {
							streamed = parity22ProtocolOutput(value["response"].(map[string]any)["output"])
						}
					}
				}
			}
		}
		if streamed == nil {
			t.Fatalf("Go stream did not complete: %s", item.name)
		}
		cases = append(cases, map[string]any{"name": item.name, "kind": "chat_output", "request": decode([]byte(item.request)), "input": input, "expected": parity22ProtocolOutput(decode(raw)["output"]), "chunks": chunks, "expected_stream": streamed})
	}
	raw, err := json.MarshalIndent(cases, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	raw = append(raw, '\n')
	path := os.Getenv("COCKPIT_PARITY22_PROTOCOL_FIXTURE")
	if path != "" {
		if err := os.WriteFile(path, raw, 0644); err != nil {
			t.Fatal(err)
		}
		return
	}
	stored, err := os.ReadFile("../../src-tauri/src/modules/codex_native_api/fixtures/parity22_protocol_go.json")
	if err != nil {
		t.Fatal(err)
	}
	var want, got any
	if err = json.Unmarshal(stored, &want); err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(raw, &got); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(want, got) {
		t.Fatal("Go protocol reference changed; regenerate and review the differential fixture")
	}
}

func parity22ProtocolOutput(value any) []any {
	items, _ := value.([]any)
	out := []any{}
	for _, raw := range items {
		item := raw.(map[string]any)
		result := map[string]any{"type": item["type"]}
		for _, key := range []string{"name", "namespace", "call_id", "arguments", "input"} {
			if v, ok := item[key]; ok {
				result[key] = v
			}
		}
		if item["type"] == "reasoning" {
			var text strings.Builder
			for _, raw := range item["summary"].([]any) {
				part := raw.(map[string]any)
				value, _ := part["text"].(string)
				text.WriteString(value)
			}
			result["text"] = text.String()
		}
		if item["type"] == "message" {
			var text strings.Builder
			for _, raw := range item["content"].([]any) {
				part := raw.(map[string]any)
				value, _ := part["text"].(string)
				text.WriteString(value)
			}
			result["text"] = text.String()
		}
		out = append(out, result)
	}
	return out
}
