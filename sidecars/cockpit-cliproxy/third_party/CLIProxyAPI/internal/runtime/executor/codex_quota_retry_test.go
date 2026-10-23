package executor

import (
	"net/http"
	"testing"
	"time"

	"github.com/tidwall/gjson"
)

func TestCodexQuotaIdentifiersAndWrapperResetTiming(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	for _, code := range []string{"usage_limit_reached", "insufficient_quota", "quota_exceeded", "subscription_sharing_usage_limit_exceeded"} {
		for _, field := range []string{"type", "code"} {
			for _, body := range []string{
				`{"error":{"` + field + `":"` + code + `","resets_in_seconds":123}}`,
				`{"error":{"` + field + `":"` + code + `"},"resets_at":1700000123,"resets_in_seconds":1}`,
				`{"error":{"` + field + `":"` + code + `"},"retry_after_seconds":123}`,
				`{"error":{"` + field + `":"` + code + `"},"headers":{"Retry-After":"123"}}`,
				`{"` + field + `":"` + code + `","retry_after":123}`,
			} {
				if !isCodexUsageLimitError([]byte(body)) {
					t.Fatalf("structured quota unrecognized: %s", body)
				}
				if retry := parseCodexRetryAfter(429, []byte(body), now); retry == nil || *retry != 123*time.Second {
					t.Fatalf("quota reset lost: retry=%v body=%s", retry, body)
				}
				failure := newCodexStatusErr(400, []byte(body))
				if failure.StatusCode() != 429 || !failure.IsCredentialScoped() {
					t.Fatalf("quota not credential scoped: %+v", failure)
				}
			}
		}
	}
	for _, body := range []string{
		`{"error":{"type":"invalid_request_error","code":"invalid_encrypted_content","message":"subscription_sharing_usage_limit_exceeded"}}`,
		`{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded"},"resets_in_seconds":123}`,
		`{"error":{"type":"subscription_sharing_usage_limit_exceeded","code":"context_length_exceeded"}}`,
	} {
		if isCodexUsageLimitError([]byte(body)) || parseCodexRetryAfter(429, []byte(body), now) != nil {
			t.Fatalf("caller/transient error classified as account quota: %s", body)
		}
	}
}

func TestCodexTerminalAndWebsocketSharingQuotaKeepResetHints(t *testing.T) {
	for _, event := range []string{
		`{"type":"response.failed","resets_in_seconds":123,"response":{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}}`,
		`{"type":"response.failed","response":{"retry_after_seconds":123,"error":{"type":"subscription_sharing_usage_limit_exceeded"}}}`,
		`{"type":"error","headers":{"retry-after":"123"},"error":{"code":"subscription_sharing_usage_limit_exceeded"}}`,
	} {
		err, body, ok := codexTerminalFailureErr([]byte(event))
		if !ok || err.StatusCode() != http.StatusTooManyRequests || !err.IsCredentialScoped() || err.RetryAfter() == nil || *err.RetryAfter() != 123*time.Second {
			t.Fatalf("terminal quota not classified with reset: ok=%v err=%+v body=%s", ok, err, body)
		}
		if gjson.GetBytes(body, "error.code").String() == "" && gjson.GetBytes(body, "error.type").String() == "" {
			t.Fatal("original precise quota identifier lost")
		}
	}
	for _, event := range []string{
		`{"type":"error","status":400,"resets_in_seconds":123,"error":{"code":"subscription_sharing_usage_limit_exceeded"}}`,
		`{"type":"error","status":429,"headers":{"Retry-After":"123"},"error":{"type":"subscription_sharing_usage_limit_exceeded"}}`,
		`{"type":"error","status":429,"body":{"resets_in_seconds":123,"error":{"code":"subscription_sharing_usage_limit_exceeded"}}}`,
	} {
		err, ok := parseCodexWebsocketError([]byte(event))
		if !ok {
			t.Fatalf("websocket quota lost: %s", event)
		}
		classified := err.(statusErrWithHeaders)
		if classified.StatusCode() != 429 || !classified.IsCredentialScoped() || classified.RetryAfter() == nil || *classified.RetryAfter() != 123*time.Second {
			t.Fatalf("websocket quota reset lost: %+v event=%s", classified, event)
		}
	}
}

func TestCodexWebsocketSharingQuotaKeepsDownstreamAliveForFailover(t *testing.T) {
	for _, terminal := range []string{
		sharingQuotaEvent,
		`{"type":"error","status":429,"error":{"code":"subscription_sharing_usage_limit_exceeded","resets_in_seconds":120}}`,
	} {
		notified, err := executeWebsocketStreamInSession(t, codexCreatedEvent, `{"type":"response.metadata","metadata":{}}`, terminal)
		if err == nil || statusCodeFromTestError(t, err) != 429 {
			t.Fatalf("quota should fail the attempt: %v", err)
		}
		if notified {
			t.Fatal("quota teardown closed downstream before failover could deliver a response")
		}
	}
}
