package main

import (
	"encoding/json"
	"net/http"
	"strings"
	"testing"
)

func TestProcessImageResponseFramePreservesStructuredFailure(t *testing.T) {
	frame := []byte("event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"type\":\"insufficient_balance\",\"message\":\"no credits\",\"status_code\":402}}}\n\n")
	_, done, err := processImageResponseFrame(frame, "b64_json")
	if done || err == nil {
		t.Fatalf("failure frame = done %v, err %v; want terminal error", done, err)
	}
	if statusCodeFromError(err) != http.StatusPaymentRequired || !strings.Contains(err.Error(), "insufficient_balance: no credits") {
		t.Fatalf("unexpected structured failure: status=%d err=%v", statusCodeFromError(err), err)
	}
}

func TestProcessImageResponseFramePreservesIncompleteFailure(t *testing.T) {
	frame := []byte("data: {\"type\":\"response.incomplete\",\"response\":{\"message\":\"generation stopped\"}}\n\n")
	_, done, err := processImageResponseFrame(frame, "b64_json")
	if done || err == nil || !strings.Contains(err.Error(), "generation stopped") {
		t.Fatalf("incomplete frame = done %v, err %v; want error", done, err)
	}
}

func TestImageResponseTerminalErrorDoesNotUseUnstructuredPromptText(t *testing.T) {
	event := map[string]any{"type": "response.failed", "response": map[string]any{
		"error": map[string]any{"code": "server_error", "message": "upstream rejected image"},
	}}
	payload, _ := json.Marshal(event)
	_, _, err := processImageResponseFrame(append([]byte("data: "), append(payload, []byte("\n\n")...)...), "b64_json")
	if err == nil || !strings.Contains(err.Error(), "server_error: upstream rejected image") {
		t.Fatalf("unexpected error: %v", err)
	}
}
