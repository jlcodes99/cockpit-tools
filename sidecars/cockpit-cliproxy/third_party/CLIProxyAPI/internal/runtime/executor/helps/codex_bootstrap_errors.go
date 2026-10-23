package helps

import (
	"net/http"
	"strings"

	"github.com/tidwall/gjson"
)

// CodexBootstrapAccountFailureStatus recognizes only structured error codes.
// This function must only receive a terminal error envelope, never output text.
func CodexBootstrapAccountFailureStatus(body []byte) (int, bool) {
	if !gjson.ValidBytes(body) {
		return 0, false
	}
	code := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.code").String()))
	kind := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.type").String()))
	if CodexBootstrapInputError(body) {
		return 0, false
	}
	for _, value := range []string{code, kind} {
		switch value {
		case "usage_limit_reached", "insufficient_quota", "quota_exceeded", "subscription_sharing_usage_limit_exceeded":
			return http.StatusTooManyRequests, true
		case "account_deactivated", "account_disabled", "account_suspended", "organization_deactivated":
			return http.StatusForbidden, true
		}
	}
	return 0, false
}

// Specific caller/input fault identifiers win over conflicting account or
// capacity hints. A generic invalid_request_error type alone is insufficient.
func CodexBootstrapInputError(body []byte) bool {
	for _, path := range []string{"error.code", "error.type", "code", "type"} {
		switch strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, path).String())) {
		case "context_length_exceeded", "context_too_large", "invalid_value", "invalid_argument", "invalid_encrypted_content", "thinking_signature_invalid", "content_policy_violation", "cyber_policy", "invalid_prompt", "unsupported_value", "message_too_big", "string_above_max_length", "previous_response_not_found":
			return true
		}
	}
	return false
}

// CodexQuotaExhaustion accepts only structured error identifiers. Specific
// caller faults win over conflicting quota hints, just as in bootstrap routing.
func CodexQuotaExhaustion(body []byte) bool {
	if !gjson.ValidBytes(body) || CodexBootstrapInputError(body) {
		return false
	}
	for _, path := range []string{"error.type", "error.code", "type", "code"} {
		switch strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, path).String())) {
		case "usage_limit_reached", "insufficient_quota", "quota_exceeded", "subscription_sharing_usage_limit_exceeded":
			return true
		}
	}
	return false
}
