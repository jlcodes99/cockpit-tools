package executor

import (
	"net/http"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	"github.com/tidwall/gjson"
)

func TestCodexContextErrorRetainsClientRecoveryCode(t *testing.T) {
	const message = "Your input exceeds the context window of this model. Please adjust your input and try again."
	for _, event := range []string{
		`{"type":"error","error":{"code":"context_length_exceeded","message":"` + message + `"}}`,
		`{"type":"response.failed","response":{"error":{"code":"context_length_exceeded","message":"` + message + `"}}}`,
		`{"type":"error","code":"context_length_exceeded","message":"` + message + `"}`,
		`{"type":"error","error":{"code":"context_too_large","message":"` + message + `"}}`,
	} {
		t.Run(event, func(t *testing.T) {
			err, _, ok := codexTerminalFailureErr([]byte(event))
			if !ok {
				t.Fatal("expected terminal context error")
			}
			if got := err.StatusCode(); got != http.StatusBadRequest {
				t.Fatalf("status = %d, want 400", got)
			}
			// Exercise the actual terminal event builder: Codex recognizes this
			// exact code, otherwise it retries the stream instead of recovering.
			eventName, chunk := handlers.BuildOpenAIResponsesStreamTerminalEvent(err.StatusCode(), err, 2)
			if eventName != "response.failed" || gjson.GetBytes(chunk, "type").String() != eventName {
				t.Fatalf("unexpected terminal event: %s %s", eventName, chunk)
			}
			if got := gjson.GetBytes(chunk, "response.error.code").String(); got != "context_length_exceeded" {
				t.Fatalf("client recovery code = %q, want context_length_exceeded; event=%s", got, chunk)
			}
			if got := gjson.GetBytes(chunk, "response.error.message").String(); got != message {
				t.Fatalf("upstream message changed: %q", got)
			}
		})
	}
}
