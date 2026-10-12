package executor

import (
	"context"
	"strings"
)

type serviceTierDefaultProviderKey struct{}
type serviceTierDefaultSnapshotKey struct{}

// WithServiceTierDefaultProvider supplies a host-owned default for new requests.
// The provider must be safe to read concurrently. An empty value means no tier default.
func WithServiceTierDefaultProvider(ctx context.Context, provider func() string) context.Context {
	if ctx == nil {
		ctx = context.Background()
	}
	if provider == nil {
		return ctx
	}
	return context.WithValue(ctx, serviceTierDefaultProviderKey{}, provider)
}

// SnapshotServiceTierDefault freezes the host default for one request and its retries.
// Long-lived connections should call this separately for every new turn.
func SnapshotServiceTierDefault(ctx context.Context) context.Context {
	if ctx == nil {
		ctx = context.Background()
	}
	if _, ok := ServiceTierDefaultSnapshot(ctx); ok {
		return ctx
	}
	provider, _ := ctx.Value(serviceTierDefaultProviderKey{}).(func() string)
	if provider == nil {
		return ctx
	}
	return context.WithValue(ctx, serviceTierDefaultSnapshotKey{}, strings.TrimSpace(provider()))
}

// ServiceTierDefaultSnapshot returns the request default, including an explicit
// empty snapshot that suppresses a previously configured startup default.
func ServiceTierDefaultSnapshot(ctx context.Context) (string, bool) {
	if ctx == nil {
		return "", false
	}
	tier, ok := ctx.Value(serviceTierDefaultSnapshotKey{}).(string)
	return tier, ok
}
