package executor

import (
	"context"
	"errors"
	"testing"
)

func TestUpstreamSendGuardPreservesTypedErrorAndSelectedIdentity(t *testing.T) {
	rejected := errors.New("local quota reservation")
	calls := 0
	ctx := WithUpstreamSendGuard(nil, func(_ context.Context, authID string) error {
		calls++
		if authID != "selected-auth" {
			t.Fatalf("authID=%q", authID)
		}
		return rejected
	})
	err := CheckUpstreamSend(ctx, "selected-auth")
	if !errors.Is(err, rejected) || !IsUpstreamSendGuardError(err) || calls != 1 {
		t.Fatalf("err=%v calls=%d", err, calls)
	}
	if IsUpstreamSendGuardError(rejected) {
		t.Fatal("ordinary transport error was marked as local")
	}
	if err := CheckUpstreamSend(context.Background(), "unprotected"); err != nil {
		t.Fatal(err)
	}
}

func TestUpstreamSendGuardCancellationDoesNotDispatch(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	ctx = WithUpstreamSendGuard(ctx, func(context.Context, string) error {
		t.Fatal("cancelled request reached admission")
		return nil
	})
	cancel()
	if err := CheckUpstreamSend(ctx, "auth"); !errors.Is(err, context.Canceled) || !IsUpstreamSendGuardError(err) {
		t.Fatalf("err=%v", err)
	}
}
