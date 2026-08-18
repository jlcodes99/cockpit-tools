package handlers

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"testing"

	"github.com/tidwall/gjson"
)

type responsesStreamEventTestError struct {
	event []byte
}

func (e responsesStreamEventTestError) Error() string { return "stream failed" }
func (e responsesStreamEventTestError) ResponsesStreamEvent() []byte {
	return e.event
}

func TestBuildOpenAIResponsesStreamErrorChunk(t *testing.T) {
	chunk := BuildOpenAIResponsesStreamErrorChunk(http.StatusInternalServerError, "unexpected EOF", 0)
	var payload map[string]any
	if err := json.Unmarshal(chunk, &payload); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if payload["type"] != "error" {
		t.Fatalf("type = %v, want %q", payload["type"], "error")
	}
	if payload["code"] != "internal_server_error" {
		t.Fatalf("code = %v, want %q", payload["code"], "internal_server_error")
	}
	if payload["message"] != "unexpected EOF" {
		t.Fatalf("message = %v, want %q", payload["message"], "unexpected EOF")
	}
	if payload["sequence_number"] != float64(0) {
		t.Fatalf("sequence_number = %v, want %v", payload["sequence_number"], 0)
	}
}

func TestBuildOpenAIResponsesStreamErrorChunkExtractsHTTPErrorBody(t *testing.T) {
	chunk := BuildOpenAIResponsesStreamErrorChunk(
		http.StatusInternalServerError,
		`{"error":{"message":"oops","type":"server_error","code":"internal_server_error"}}`,
		0,
	)
	var payload map[string]any
	if err := json.Unmarshal(chunk, &payload); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if payload["type"] != "error" {
		t.Fatalf("type = %v, want %q", payload["type"], "error")
	}
	if payload["code"] != "internal_server_error" {
		t.Fatalf("code = %v, want %q", payload["code"], "internal_server_error")
	}
	if payload["message"] != "oops" {
		t.Fatalf("message = %v, want %q", payload["message"], "oops")
	}
}

func TestBuildOpenAIResponsesStreamTerminalEventPreservesResponseFailed(t *testing.T) {
	want := []byte(`{"type":"response.failed","response":{"status":"failed","error":{"type":"service_unavailable_error","code":"server_is_overloaded","message":"overloaded"}}}`)
	eventName, payload := BuildOpenAIResponsesStreamTerminalEvent(
		http.StatusServiceUnavailable,
		fmt.Errorf("wrapped: %w", responsesStreamEventTestError{event: want}),
		0,
	)
	if eventName != "response.failed" {
		t.Fatalf("event name = %q, want response.failed", eventName)
	}
	if !bytes.Equal(payload, want) {
		t.Fatalf("payload = %s, want %s", payload, want)
	}
}

func TestBuildOpenAIResponsesStreamTerminalEventFallsBackToValidError(t *testing.T) {
	eventName, payload := BuildOpenAIResponsesStreamTerminalEvent(http.StatusBadGateway, errors.New("upstream closed"), 0)
	if eventName != "error" {
		t.Fatalf("event name = %q, want error", eventName)
	}
	if got := gjson.GetBytes(payload, "type").String(); got != "error" {
		t.Fatalf("payload type = %q, want error: %s", got, payload)
	}
}
