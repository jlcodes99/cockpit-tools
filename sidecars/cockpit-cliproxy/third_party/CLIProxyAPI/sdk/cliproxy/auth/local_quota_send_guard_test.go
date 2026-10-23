package auth

import (
	"context"
	"testing"
	"time"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestLocalQuotaSendGuardRejectionAllowsRotationWithoutCoolingCredential(t *testing.T) {
	local := &Error{Code: "quota_reserved", Message: "quota changed before send", HTTPStatus: 429}
	ctx := cliproxyexecutor.WithUpstreamSendGuard(context.Background(), func(context.Context, string) error { return local })
	rejected := cliproxyexecutor.CheckUpstreamSend(ctx, "auth")
	result := resultErrorFromError(rejected)
	if result.Code != local.Code || isRequestInvalidError(rejected) || !shouldSkipCredentialCooldown(result) {
		t.Fatalf("local rejection lost its admission classification: %#v", result)
	}
	auth := &Auth{ID: "auth"}
	applyAuthFailureState(auth, result, nil, time.Now(), false)
	if auth.Unavailable || auth.Quota.Exceeded || !auth.NextRetryAfter.IsZero() {
		t.Fatalf("unsent local request cooled credential: %#v", auth)
	}
	if shouldSkipCredentialCooldown(&Error{Code: "rate_limit_exceeded", HTTPStatus: 429}) {
		t.Fatal("real upstream rate limit must still cool credential")
	}
}
