package responses

import "testing"

func TestParseInputToItems_PreservesFunctionFields(t *testing.T) {
	items, err := parseInputToItems([]interface{}{
		map[string]interface{}{
			"type":      "function_call",
			"id":        "fc_call_1",
			"status":    "completed",
			"call_id":   "call_1",
			"name":      "get_weather",
			"arguments": `{"location":"NYC"}`,
		},
		map[string]interface{}{
			"type":    "function_call_output",
			"call_id": "call_1",
			"output":  "Sunny",
		},
	})
	if err != nil {
		t.Fatalf("parseInputToItems failed: %v", err)
	}
	if len(items) != 2 {
		t.Fatalf("expected 2 items, got %d", len(items))
	}
	if items[0].CallID != "call_1" || items[0].Name != "get_weather" {
		t.Fatalf("function_call fields not preserved: %#v", items[0])
	}
	if items[1].CallID != "call_1" || items[1].Output != "Sunny" {
		t.Fatalf("function_call_output fields not preserved: %#v", items[1])
	}
}

func TestParseInputToItems_NormalizesLegacyToolItems(t *testing.T) {
	items, err := parseInputToItems([]interface{}{
		map[string]interface{}{
			"type": "tool_call",
			"tool_use": map[string]interface{}{
				"id":   "toolu_1",
				"name": "get_weather",
				"input": map[string]interface{}{
					"location": "NYC",
				},
			},
		},
		map[string]interface{}{
			"type": "tool_result",
			"content": map[string]interface{}{
				"tool_use_id": "toolu_1",
				"content":     map[string]interface{}{"temperature": 72},
			},
		},
	})
	if err != nil {
		t.Fatalf("parseInputToItems failed: %v", err)
	}
	if len(items) != 2 {
		t.Fatalf("expected 2 items, got %d", len(items))
	}
	if items[0].Type != "function_call" || items[0].CallID != "toolu_1" || items[0].Name != "get_weather" {
		t.Fatalf("legacy tool_call not normalized: %#v", items[0])
	}
	if items[0].Arguments == "" {
		t.Fatalf("legacy tool_call arguments not preserved: %#v", items[0])
	}
	if items[1].Type != "function_call_output" || items[1].CallID != "toolu_1" {
		t.Fatalf("legacy tool_result not normalized: %#v", items[1])
	}
	outputMap, ok := items[1].Output.(map[string]interface{})
	if !ok || outputMap["temperature"] != 72 {
		t.Fatalf("legacy tool_result output not preserved: %#v", items[1].Output)
	}
}
