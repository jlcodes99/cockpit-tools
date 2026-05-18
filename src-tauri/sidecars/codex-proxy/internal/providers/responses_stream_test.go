package providers

import (
	"encoding/json"
	"io"
	"strings"
	"testing"
)

func extractInputJSONDelta(t *testing.T, events []string) string {
	t.Helper()
	for _, event := range events {
		for _, line := range strings.Split(event, "\n") {
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			jsonStr := strings.TrimPrefix(line, "data: ")

			var data map[string]interface{}
			if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
				continue
			}
			if data["type"] != "content_block_delta" {
				continue
			}
			delta, ok := data["delta"].(map[string]interface{})
			if !ok {
				continue
			}
			if delta["type"] != "input_json_delta" {
				continue
			}
			if partial, ok := delta["partial_json"].(string); ok && partial != "" {
				return partial
			}
		}
	}

	t.Fatalf("input_json_delta not found, events=%v", events)
	return ""
}

func extractMessageDeltaUsage(t *testing.T, events []string) map[string]interface{} {
	t.Helper()
	for _, event := range events {
		for _, line := range strings.Split(event, "\n") {
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			jsonStr := strings.TrimPrefix(line, "data: ")

			var data map[string]interface{}
			if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
				continue
			}
			if data["type"] != "message_delta" {
				continue
			}
			if usage, ok := data["usage"].(map[string]interface{}); ok {
				return usage
			}
		}
	}
	t.Fatalf("message_delta usage not found, events=%v", events)
	return nil
}

func extractMessageStartUsage(t *testing.T, events []string) map[string]interface{} {
	t.Helper()
	for _, event := range events {
		for _, line := range strings.Split(event, "\n") {
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			jsonStr := strings.TrimPrefix(line, "data: ")

			var data map[string]interface{}
			if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
				continue
			}
			if data["type"] != "message_start" {
				continue
			}
			message, ok := data["message"].(map[string]interface{})
			if !ok {
				continue
			}
			if usage, ok := message["usage"].(map[string]interface{}); ok {
				return usage
			}
		}
	}
	t.Fatalf("message_start usage not found, events=%v", events)
	return nil
}

func TestResponsesProvider_HandleStreamResponse_StripsEmptyReadPages(t *testing.T) {
	body := `event: response.output_item.added
data: {"type":"response.output_item.added","item":{"type":"function_call","call_id":"call_1","name":"Read"}}

event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","delta":"{\"file_path\":\"/tmp/x\",\"pages\":\"\"}"}

event: response.output_item.done
data: {"type":"response.output_item.done","item":{"type":"function_call","call_id":"call_1","name":"Read","arguments":"{\"file_path\":\"/tmp/x\",\"pages\":\"\"}"}}

event: response.completed
data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":1,"output_tokens":1}}}

`

	provider := &ResponsesProvider{}
	eventChan, errChan, err := provider.HandleStreamResponse(io.NopCloser(strings.NewReader(body)))
	if err != nil {
		t.Fatalf("HandleStreamResponse returned error: %v", err)
	}

	events := collectStreamEvents(eventChan)
	select {
	case streamErr := <-errChan:
		if streamErr != nil {
			t.Fatalf("unexpected stream error: %v", streamErr)
		}
	default:
	}

	partialJSON := extractInputJSONDelta(t, events)
	var input map[string]interface{}
	if err := json.Unmarshal([]byte(partialJSON), &input); err != nil {
		t.Fatalf("partial_json is not valid JSON: %v, partial_json=%q", err, partialJSON)
	}
	if _, exists := input["pages"]; exists {
		t.Fatalf("pages exists = true, want false; input=%v", input)
	}
	if input["file_path"] != "/tmp/x" {
		t.Fatalf("file_path = %v, want /tmp/x", input["file_path"])
	}
}

func TestResponsesProvider_HandleStreamResponse_PropagatesCacheUsageFromInputTokensDetails(t *testing.T) {
	body := `event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hello"}

event: response.completed
data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":5,"input_tokens_details":{"cached_tokens":7},"cache_creation_5m_input_tokens":2,"cache_ttl":"5m"}}}

`

	provider := &ResponsesProvider{}
	eventChan, errChan, err := provider.HandleStreamResponse(io.NopCloser(strings.NewReader(body)))
	if err != nil {
		t.Fatalf("HandleStreamResponse returned error: %v", err)
	}

	events := collectStreamEvents(eventChan)
	select {
	case streamErr := <-errChan:
		if streamErr != nil {
			t.Fatalf("unexpected stream error: %v", streamErr)
		}
	default:
	}

	usage := extractMessageDeltaUsage(t, events)
	if int(usage["input_tokens"].(float64)) != 10 || int(usage["output_tokens"].(float64)) != 5 {
		t.Fatalf("basic usage mismatch: %#v", usage)
	}
	if int(usage["cache_read_input_tokens"].(float64)) != 7 {
		t.Fatalf("cache_read_input_tokens mismatch: %#v", usage)
	}
	if int(usage["cache_creation_5m_input_tokens"].(float64)) != 2 {
		t.Fatalf("cache_creation_5m_input_tokens mismatch: %#v", usage)
	}
	if usage["cache_ttl"] != "5m" {
		t.Fatalf("cache_ttl mismatch: %#v", usage)
	}
}

func TestResponsesProvider_HandleStreamResponse_LeavesResponsesTotalPromptTokensInUsage(t *testing.T) {
	body := `event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hello"}

event: response.completed
data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":114931,"output_tokens":100,"cache_read_input_tokens":112256}}}

`

	provider := &ResponsesProvider{}
	eventChan, errChan, err := provider.HandleStreamResponse(io.NopCloser(strings.NewReader(body)))
	if err != nil {
		t.Fatalf("HandleStreamResponse returned error: %v", err)
	}

	events := collectStreamEvents(eventChan)
	select {
	case streamErr := <-errChan:
		if streamErr != nil {
			t.Fatalf("unexpected stream error: %v", streamErr)
		}
	default:
	}

	messageStartUsage := extractMessageStartUsage(t, events)
	if int(messageStartUsage["input_tokens"].(float64)) != 0 {
		t.Fatalf("message_start input_tokens = %v, want 0 placeholder", messageStartUsage["input_tokens"])
	}

	usage := extractMessageDeltaUsage(t, events)
	if int(usage["input_tokens"].(float64)) != 114931 {
		t.Fatalf("input_tokens = %v, want 114931", usage["input_tokens"])
	}
	if int(usage["cache_read_input_tokens"].(float64)) != 112256 {
		t.Fatalf("cache_read_input_tokens = %v, want 112256", usage["cache_read_input_tokens"])
	}
}

func TestResponsesProvider_HandleStreamResponse_DoesNotInventMessageStartPromptTotals(t *testing.T) {
	body := `event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hello"}

event: response.completed
data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":1,"output_tokens":5,"input_tokens_details":{"cached_tokens":112256}}}}

`

	provider := &ResponsesProvider{}
	eventChan, errChan, err := provider.HandleStreamResponse(io.NopCloser(strings.NewReader(body)))
	if err != nil {
		t.Fatalf("HandleStreamResponse returned error: %v", err)
	}

	events := collectStreamEvents(eventChan)
	select {
	case streamErr := <-errChan:
		if streamErr != nil {
			t.Fatalf("unexpected stream error: %v", err)
		}
	default:
	}

	messageStartUsage := extractMessageStartUsage(t, events)
	if int(messageStartUsage["input_tokens"].(float64)) != 0 {
		t.Fatalf("message_start input_tokens = %v, want 0 placeholder", messageStartUsage["input_tokens"])
	}

	usage := extractMessageDeltaUsage(t, events)
	if int(usage["input_tokens"].(float64)) != 1 {
		t.Fatalf("message_delta input_tokens = %v, want 1", usage["input_tokens"])
	}
	if int(usage["cache_read_input_tokens"].(float64)) != 112256 {
		t.Fatalf("cache_read_input_tokens = %v, want 112256", usage["cache_read_input_tokens"])
	}
}
