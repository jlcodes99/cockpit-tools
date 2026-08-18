package openai

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestNormalizeResponsesLiteRequestDisablesParallelTools(t *testing.T) {
	request := []byte(`{"parallel_tool_calls":true}`)
	result := normalizeResponsesLiteRequest(request, true)
	if got := gjson.GetBytes(result, "parallel_tool_calls"); !got.Exists() || got.Bool() {
		t.Fatalf("parallel_tool_calls = %s, want false", got.Raw)
	}
}

func TestNormalizeResponsesLiteRequestLeavesRegularRequestsUnchanged(t *testing.T) {
	request := []byte(`{"model":"gpt-5.5","parallel_tool_calls":true}`)
	result := normalizeResponsesLiteRequest(request, false)
	if string(result) != string(request) {
		t.Fatalf("regular request changed: %s", result)
	}
}

func TestNormalizeResponsesLiteRequestUsesRegistryModelWithoutHeader(t *testing.T) {
	request := []byte(`{"model":"gpt-5.6-luna","parallel_tool_calls":true}`)
	result := normalizeResponsesLiteRequest(request, false)
	if got := gjson.GetBytes(result, "parallel_tool_calls"); !got.Exists() || got.Bool() {
		t.Fatalf("parallel_tool_calls = %s, want false", got.Raw)
	}
}
