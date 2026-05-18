package converters

import (
	"context"
	"strings"
	"testing"
)

func TestConvertClaudeMessagesToResponses_StreamThinkingToReasoning(t *testing.T) {
	lines := []string{
		`data: {"type":"message_start","message":{"id":"msg_ds","type":"message","role":"assistant","model":"deepseek-v4-pro","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}`,
		`data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}`,
		`data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"messages thinking"}}`,
		`data: {"type":"content_block_stop","index":0}`,
		`data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}`,
		`data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"messages text"}}`,
		`data: {"type":"content_block_stop","index":1}`,
		`data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"input_tokens":1,"output_tokens":2}}`,
		`data: {"type":"message_stop"}`,
	}

	var state any
	var events []string
	for _, line := range lines {
		events = append(events, ConvertClaudeMessagesToResponses(context.Background(), "deepseek-v4-pro", []byte(`{"model":"deepseek-v4-pro","input":"hello"}`), nil, []byte(line), &state)...)
	}

	joined := strings.Join(events, "\n")
	if !strings.Contains(joined, `"type":"reasoning"`) {
		t.Fatalf("expected reasoning item events, got %v", events)
	}
	if !strings.Contains(joined, `"text":"messages thinking"`) {
		t.Fatalf("expected reasoning text, got %v", events)
	}
	if !strings.Contains(joined, `"delta":"messages text"`) {
		t.Fatalf("expected output text delta, got %v", events)
	}
	if !strings.Contains(joined, `"type":"response.completed"`) {
		t.Fatalf("expected response.completed, got %v", events)
	}
}

func TestConvertClaudeMessagesToResponses_StreamToolUseToFunctionCall(t *testing.T) {
	lines := []string{
		`data: {"type":"message_start","message":{"id":"msg_tool","type":"message","role":"assistant","model":"deepseek-v4-pro","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}`,
		`data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_read","name":"Read","input":{}}}`,
		`data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\""}}`,
		`data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":":\"/tmp/a\"}"}}`,
		`data: {"type":"content_block_stop","index":0}`,
		`data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"input_tokens":1,"output_tokens":2}}`,
		`data: {"type":"message_stop"}`,
	}

	var state any
	var events []string
	for _, line := range lines {
		events = append(events, ConvertClaudeMessagesToResponses(context.Background(), "deepseek-v4-pro", []byte(`{"model":"deepseek-v4-pro","input":"hello"}`), nil, []byte(line), &state)...)
	}

	joined := strings.Join(events, "\n")
	for _, want := range []string{
		`"type":"function_call"`,
		`"call_id":"call_read"`,
		`"name":"Read"`,
		`"arguments":"{\"file_path\":\"/tmp/a\"}"`,
		`response.function_call_arguments.delta`,
		`response.function_call_arguments.done`,
		`"type":"response.completed"`,
	} {
		if !strings.Contains(joined, want) {
			t.Fatalf("expected %q in converted events, got %v", want, events)
		}
	}
}
