package executor

import (
	"net/http"
	"testing"

	"github.com/tidwall/gjson"
)

func TestCodexV72157CapacityTriggersBootstrapFailover(t *testing.T) {
	for _, body := range []string{
		`{"error":{"code":"model_at_capacity","message":"try later"}}`,
		`{"error":{"message":"The requested model is at capacity"}}`,
	} {
		if !isCodexModelCapacityError([]byte(body)) {
			t.Fatalf("capacity error was not classified: %s", body)
		}
		if !isCodexOverloadBootstrapFailure([]byte(body)) {
			t.Fatalf("capacity error was not eligible for bootstrap failover: %s", body)
		}
	}
}

func TestCodexV72157ModelNotFoundPrecedesInvalidRequest(t *testing.T) {
	body := []byte(`{"error":{"type":"invalid_request_error","code":"model_not_found","message":"missing"}}`)
	if got := codexTerminalFailureStatus(body); got != http.StatusNotFound {
		t.Fatalf("status = %d, want %d", got, http.StatusNotFound)
	}
}

func TestCodexV72157PreservesTerminalSequenceNumber(t *testing.T) {
	event := []byte(`{"type":"response.failed","sequence_number":17,"response":{"error":{"type":"server_error","code":"failed","message":"boom"}}}`)
	body, ok := codexTerminalFailureBody(event)
	if !ok {
		t.Fatal("terminal event was not recognized")
	}
	if got := gjson.GetBytes(body, "sequence_number").Int(); got != 17 {
		t.Fatalf("sequence_number = %d, want 17; body=%s", got, body)
	}
}

func TestCodexV72157ModelLevelCoolingKeepsCredentialAvailable(t *testing.T) {
	body := []byte(`{"error":{"code":"usage_limit_reached","message":"usage limit reached"}}`)
	err := newCodexStatusErrWithCooling(http.StatusTooManyRequests, body, true)
	if err.StatusCode() != http.StatusTooManyRequests {
		t.Fatalf("status = %d, want %d", err.StatusCode(), http.StatusTooManyRequests)
	}
	if err.IsCredentialScoped() {
		t.Fatal("model-level cooling unexpectedly marked the entire credential unavailable")
	}
}
