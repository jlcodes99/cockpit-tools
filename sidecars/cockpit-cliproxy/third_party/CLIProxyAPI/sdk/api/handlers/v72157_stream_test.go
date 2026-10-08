package handlers

import (
	"encoding/json"
	"errors"
	"net/http"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestV72157SSEValidationHandlesSplitCRLF(t *testing.T) {
	state := &sseJSONValidationState{}
	chunks := []string{
		"data: {\"type\":\"response.completed\",\r",
		"\ndata: \"response\":{\"status\":\"completed\"}}\r\n\r\n",
	}
	var output []byte
	for _, chunk := range chunks {
		out, err := state.AddChunk([]byte(chunk))
		if err != nil {
			t.Fatalf("AddChunk: %v", err)
		}
		output = append(output, out...)
	}
	if err := state.Finish(); err != nil {
		t.Fatalf("Finish: %v", err)
	}
	want := "data: {\"type\":\"response.completed\",\ndata: \"response\":{\"status\":\"completed\"}}\n\n"
	if string(output) != want {
		t.Fatalf("output = %q, want %q", output, want)
	}
}

func TestV72157ResponsesErrorPreservesNestedDetailsAndSequence(t *testing.T) {
	errText := `{"error":{"type":"invalid_request","code":"cyber_policy","message":"blocked","param":null,"detail":{"reason":"policy"}},"sequence_number":7}`
	chunk := BuildOpenAIResponsesStreamErrorChunk(http.StatusBadRequest, errText, 2)
	var payload struct {
		Type           string         `json:"type"`
		Error          map[string]any `json:"error"`
		SequenceNumber int            `json:"sequence_number"`
	}
	if err := json.Unmarshal(chunk, &payload); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if payload.Type != "error" || payload.SequenceNumber != 7 {
		t.Fatalf("payload = %#v", payload)
	}
	if payload.Error["code"] != "cyber_policy" || payload.Error["detail"] == nil {
		t.Fatalf("nested error details were not preserved: %#v", payload.Error)
	}
}

func TestV72157TerminalAuthResponseIsNonRetryable(t *testing.T) {
	err := coreauth.NewTerminalAuthError(&coreauth.Error{
		Code:       "auth_unavailable",
		Message:    "no auth available",
		HTTPStatus: http.StatusServiceUnavailable,
	}, errors.New("refresh token revoked"))
	body := BuildErrorResponseBodyWithError(http.StatusServiceUnavailable, err.Error(), err)
	var payload ErrorResponse
	if errUnmarshal := json.Unmarshal(body, &payload); errUnmarshal != nil {
		t.Fatalf("unmarshal: %v", errUnmarshal)
	}
	if payload.Error.Code != "upstream_authentication_required" || payload.Error.Retryable == nil || *payload.Error.Retryable {
		t.Fatalf("unexpected terminal auth response: %#v", payload.Error)
	}
}
