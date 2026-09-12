package main

import (
	"net/http"
	"testing"
)

func TestUsageAttributionStaysWithRecordedAttempt(t *testing.T) {
	for _, laterSelection := range []bool{false, true} {
		t.Run(map[bool]string{false: "selection_missing", true: "selection_changed"}[laterSelection], func(t *testing.T) {
			tracker := newRequestUsageTracker()
			tracker.record(usagePayload{RequestID: "attribution", AccountID: "member-a-space-one", AccountEmail: "a@example.invalid", AuthID: "auth-a", Success: true,
				Usage: usageDetails{InputTokens: 120, OutputTokens: 30, TotalTokens: 150}})
			if laterSelection {
				tracker.recordSelectedAccount("attribution", &accountSpec{ID: "member-b-space-one", Email: "b@example.invalid"}, "auth-b")
			}
			result, ok := tracker.finalize("attribution", usageFinalizeInput{status: http.StatusOK})
			if !ok || result.AccountID != "member-a-space-one" || result.AuthID != "auth-a" || result.AccountEmail != "a@example.invalid" || result.Usage.TotalTokens != 150 {
				t.Fatalf("usage attribution was overwritten: %+v", result)
			}
		})
	}
}

func TestUnmappedUsageDoesNotBorrowDifferentSelectedAuth(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.recordSelectedAccount("unknown", &accountSpec{ID: "account-b"}, "auth-b")
	tracker.record(usagePayload{RequestID: "unknown", AuthID: "auth-a", Success: true, Usage: usageDetails{TotalTokens: 150}})
	result, _ := tracker.finalize("unknown", usageFinalizeInput{status: http.StatusOK})
	if result.AccountID != "" || result.AuthID != "auth-a" {
		t.Fatalf("unknown usage must remain unattributed instead of charging account-b: %+v", result)
	}
}

func TestMatchingSelectedAuthCanFillUnmappedUsage(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.recordSelectedAccount("matched", &accountSpec{ID: "member-a-space-one", Email: "a@example.invalid"}, "auth-a")
	tracker.record(usagePayload{RequestID: "matched", AuthID: "auth-a", Success: true, Usage: usageDetails{TotalTokens: 150}})
	result, _ := tracker.finalize("matched", usageFinalizeInput{status: http.StatusOK})
	if result.AccountID != "member-a-space-one" || result.AuthID != "auth-a" || result.Usage.TotalTokens != 150 {
		t.Fatalf("matching selector should fill missing account mapping: %+v", result)
	}
}

func TestLastSuccessfulUsageKeepsItsOwnAccount(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.record(usagePayload{RequestID: "successes", AccountID: "space-a", AuthID: "auth-a", Success: true, Usage: usageDetails{TotalTokens: 10}})
	tracker.record(usagePayload{RequestID: "successes", AccountID: "space-b", AuthID: "auth-b", Success: true, Usage: usageDetails{TotalTokens: 20}})
	tracker.recordSelectedAccount("successes", &accountSpec{ID: "space-c"}, "auth-c")
	result, _ := tracker.finalize("successes", usageFinalizeInput{status: http.StatusOK})
	if result.AccountID != "space-b" || result.AuthID != "auth-b" || result.Usage.TotalTokens != 20 {
		t.Fatalf("final selected usage and account must stay paired: %+v", result)
	}
}

func TestAPIKeyUsageWithoutAuthIDPreservesAccount(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.recordSelectedAccount("apikey", &accountSpec{ID: "other-account"}, "other-auth")
	tracker.record(usagePayload{RequestID: "apikey", AccountID: "api-account", Success: true, Usage: usageDetails{TotalTokens: 25}})
	result, _ := tracker.finalize("apikey", usageFinalizeInput{status: http.StatusOK, spec: &apiKeySpec{ID: "client-key", Label: "Test"}})
	if result.AccountID != "api-account" || result.AuthID != "" || result.APIKeyID != "client-key" || result.Usage.TotalTokens != 25 {
		t.Fatalf("API account and client API key are different identities: %+v", result)
	}
}
