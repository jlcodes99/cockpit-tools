package auth

import (
	"context"
	"strings"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

const attemptedAuthMetadataKey = "cliproxy.internal.attempted_auths"

// Codex transports track their send boundary. Other SDK executors may predate
// that contract, so only explicit local guard failures are unsent for them.
func isUnsentExecutionFailure(ctx context.Context, provider string, err error) bool {
	if err == nil || cliproxyexecutor.UpstreamAttempted(ctx) || hasUpstreamExecutionAttempt(err) {
		return false
	}
	return cliproxyexecutor.IsUpstreamSendGuardError(err) ||
		(strings.EqualFold(strings.TrimSpace(provider), "codex") && !upstreamAttemptStartedAt(ctx).IsZero())
}

func skipUnsentExecutionResult(ctx context.Context, result Result) bool {
	return result.unsent || (!result.Success &&
		strings.EqualFold(strings.TrimSpace(result.Provider), "codex") &&
		!upstreamAttemptStartedAt(ctx).IsZero() && !cliproxyexecutor.UpstreamAttempted(ctx))
}

func recordCredentialAttempt(attempted map[string]struct{}, opts cliproxyexecutor.Options, authID string, ctx context.Context, provider string, err error) {
	if isUnsentExecutionFailure(ctx, provider, err) {
		return
	}
	attempted[authID] = struct{}{}
	if roundAttempted, ok := opts.Metadata[attemptedAuthMetadataKey].(map[string]struct{}); ok && roundAttempted != nil {
		roundAttempted[authID] = struct{}{}
	}
}
