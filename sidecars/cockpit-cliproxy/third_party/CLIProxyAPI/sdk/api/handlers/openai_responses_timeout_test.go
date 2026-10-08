package handlers

import (
	"errors"
	"github.com/tidwall/gjson"
	"net/http"
	"testing"
)

func TestResponsesTimeoutErrorClassification(t *testing.T) {
	for _, status := range []int{http.StatusRequestTimeout, http.StatusGatewayTimeout} {
		_, payload := BuildOpenAIResponsesStreamTerminalEvent(status, errors.New("upstream timeout"), 0)
		if got := gjson.GetBytes(payload, "response.error.type").String(); got != "server_error" {
			t.Fatalf("status %d classified as %q: %s", status, got, payload)
		}
	}
	payload := BuildOpenAIResponsesStreamFailedChunk(http.StatusRequestTimeout, `{"error":{"code":"custom_timeout","type":"custom_type","message":"detail"}}`, 0)
	if gjson.GetBytes(payload, "response.error.code").String() != "custom_timeout" || gjson.GetBytes(payload, "response.error.type").String() != "custom_type" {
		t.Fatalf("upstream detail lost: %s", payload)
	}
}
