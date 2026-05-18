package types

import "testing"

func TestParseResponsesInput_NormalizesLegacyResponsesItemsSlice(t *testing.T) {
	items, err := ParseResponsesInput([]ResponsesItem{
		{
			Type: "tool_call",
			ToolUse: &ToolUse{
				ID:   "toolu_1",
				Name: "get_weather",
				Input: map[string]interface{}{
					"location": "NYC",
				},
			},
		},
		{
			Type: "tool_result",
			Content: map[string]interface{}{
				"tool_use_id": "toolu_1",
				"content":     map[string]interface{}{"temperature": 72},
			},
		},
	})
	if err != nil {
		t.Fatalf("ParseResponsesInput failed: %v", err)
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

func TestParseResponsesInput_PreservesCustomToolCallInput(t *testing.T) {
	items, err := ParseResponsesInput([]interface{}{
		map[string]interface{}{
			"type":    "custom_tool_call",
			"call_id": "call_1",
			"name":    "apply_patch",
			"input":   "*** Begin Patch\n*** End Patch",
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(items) != 1 {
		t.Fatalf("items len = %d", len(items))
	}
	if items[0].Input != "*** Begin Patch\n*** End Patch" {
		t.Fatalf("input = %q", items[0].Input)
	}
}

func TestParseResponsesInput_InfersMessageTypeFromRole(t *testing.T) {
	items, err := ParseResponsesInput([]interface{}{
		map[string]interface{}{
			"role":    "user",
			"content": "Who are you?",
		},
	})
	if err != nil {
		t.Fatalf("ParseResponsesInput failed: %v", err)
	}
	if len(items) != 1 {
		t.Fatalf("expected 1 item, got %d", len(items))
	}
	if items[0].Type != "message" || items[0].Role != "user" || items[0].Content != "Who are you?" {
		t.Fatalf("message type not inferred: %#v", items[0])
	}
}

func TestNormalizeResponsesItem_IsIdempotent(t *testing.T) {
	original := ResponsesItem{
		Type:      "function_call",
		CallID:    "call_1",
		Name:      "get_weather",
		Arguments: `{"location":"Tokyo"}`,
	}
	once := NormalizeResponsesItem(original)
	twice := NormalizeResponsesItem(once)

	if once != twice {
		t.Fatalf("normalize should be idempotent, once=%#v twice=%#v", once, twice)
	}
}

func TestNormalizeResponsesItem_NormalizesNestedLegacyToolCall(t *testing.T) {
	item := NormalizeResponsesItem(ResponsesItem{
		Type: "tool_call",
		Content: map[string]interface{}{
			"content": map[string]interface{}{
				"id":   "toolu_nested",
				"name": "get_weather",
				"input": map[string]interface{}{
					"location": "Paris",
				},
			},
		},
	})

	if item.Type != "function_call" {
		t.Fatalf("expected function_call, got %#v", item)
	}
	if item.CallID != "toolu_nested" || item.Name != "get_weather" {
		t.Fatalf("nested legacy tool_call not normalized: %#v", item)
	}
	if item.Arguments == "" {
		t.Fatalf("nested legacy tool_call arguments not preserved: %#v", item)
	}
}
