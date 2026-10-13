package main

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestCapacityFailurePreservesUpstreamHTTPAndStreamErrors(t *testing.T) {
	for _, status := range []int{http.StatusTooManyRequests, http.StatusBadGateway, http.StatusServiceUnavailable} {
		for _, code := range []string{"server_is_overloaded", "slow_down", "usage_limit_reached"} {
			err := relayStatusError{
				status:  status,
				message: `{"error":{"code":"` + code + `","message":"original upstream failure"}}`,
			}
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil)
			(&relayServer{}).writeExecutorError(c, err)
			wantCode := "upstream_error"
			if status == http.StatusTooManyRequests {
				wantCode = "rate_limited"
			}
			if w.Code != status || gjson.Get(w.Body.String(), "error.code").String() != wantCode ||
				gjson.Get(w.Body.String(), "error.message").String() != err.Error() {
				t.Fatalf("upstream failure changed: %d %s", w.Code, w.Body.String())
			}
			w = httptest.NewRecorder()
			c, _ = gin.CreateTestContext(w)
			writeStreamTerminalErrorForFormat(c, err, sdktranslator.FormatOpenAIResponse)
			if !strings.Contains(w.Body.String(), code) || strings.Contains(w.Body.String(), `"code":"server_error"`) {
				t.Fatalf("upstream stream error changed: %s", w.Body.String())
			}
		}
	}
}

func TestMeaningfulStreamOutputAndOverloadDetection(t *testing.T) {
	// Handshake events must NOT be treated as meaningful output
	handshakeEvents := [][]byte{
		[]byte(`{"type":"response.created","sequence_number":0,"response":{"id":"resp_1"}}`),
		[]byte(`data: {"type":"response.created","sequence_number":0}`),
		[]byte("event: response.created\n"),
		[]byte("event: response.created\ndata: {\"type\":\"response.created\",\"sequence_number\":0}\n\n"),
		[]byte(`{"type":"response.in_progress","sequence_number":1}`),
		[]byte(`data: {"type":"response.in_progress","sequence_number":1}`),
		[]byte("event: response.in_progress\n"),
		[]byte("event: response.in_progress\ndata: {\"type\":\"response.in_progress\",\"sequence_number\":1}\n\n"),
		[]byte(`: keep-alive`),
		[]byte(`data: [DONE]`),
		[]byte(`{"choices":[{"index":0,"delta":{"role":"assistant"}}]}`),
		[]byte("data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n"),
	}
	for _, event := range handshakeEvents {
		if hasMeaningfulStreamOutput(event) {
			t.Fatalf("expected event %s to not be meaningful output", string(event))
		}
	}

	// Output events MUST be treated as meaningful output
	outputEvents := [][]byte{
		[]byte(`{"type":"response.output_item.added","sequence_number":2}`),
		[]byte(`{"type":"response.output_text.delta","delta":"Hello"}`),
		[]byte(`{"type":"response.reasoning_summary_text.delta","delta":"Thinking..."}`),
		[]byte(`{"choices":[{"index":0,"delta":{"content":"Hi"}}]}`),
		[]byte(`{"choices":[{"index":0,"delta":{"reasoning_content":"Thought"}}]}`),
		[]byte(`data: {"choices":[{"index":0,"delta":{"content":"World"}}]}`),
		[]byte(`{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}`),
	}
	for _, event := range outputEvents {
		if !hasMeaningfulStreamOutput(event) {
			t.Fatalf("expected event %s to be meaningful output", string(event))
		}
	}

	// Overload rejections
	overloadPayload := []byte(`{"error":{"code":"server_is_overloaded","message":"Our servers are currently overloaded. Please try again later.","param":null,"type":"service_unavailable_error"},"sequence_number":2}`)
	if !isPayloadOverload(overloadPayload) {
		t.Fatalf("expected overloadPayload to be recognized as overload")
	}
	if hasMeaningfulStreamOutput(overloadPayload) {
		t.Fatalf("overloadPayload must not be treated as meaningful output")
	}

	// SSE wrapped overload
	sseOverload := []byte("event: error\ndata: {\"error\":{\"code\":\"server_is_overloaded\",\"message\":\"Our servers are currently overloaded. Please try again later.\",\"param\":null,\"type\":\"service_unavailable_error\"},\"sequence_number\":2}\n\n")
	if !isPayloadOverload(sseOverload) {
		t.Fatalf("expected sseOverload to be recognized as overload")
	}
	if hasMeaningfulStreamOutput(sseOverload) {
		t.Fatalf("sseOverload must not be treated as meaningful output")
	}
}

func TestOverloadErrorRecognition(t *testing.T) {
	errs := []error{
		errors.New(`{"error":{"code":"server_is_overloaded","message":"Our servers are currently overloaded. Please try again later.","param":null,"type":"service_unavailable_error"},"sequence_number":2}`),
		errors.New("HTTP 502 Bad Gateway: server_is_overloaded"),
		errors.New("service_unavailable_error"),
		errors.New("model_at_capacity"),
		relayStatusError{status: http.StatusBadGateway, message: "upstream 502"},
		relayStatusError{status: http.StatusServiceUnavailable, message: "upstream 503"},
	}
	for _, err := range errs {
		if !isOverloadError(err) {
			t.Fatalf("expected isOverloadError to be true for: %v", err)
		}
	}
}

