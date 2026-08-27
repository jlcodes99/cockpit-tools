package auth

import (
	"context"
	"testing"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func metadataAuth(id string, meta map[string]any) *Auth {
	return &Auth{
		ID:       id,
		Provider: "codebuddy",
		Metadata: meta,
	}
}

func pickOne(t *testing.T, sel Selector, auths []*Auth) *Auth {
	t.Helper()
	got, err := sel.Pick(context.Background(), "codebuddy", "deepseek-v4-flash",
		cliproxyexecutor.Options{}, auths)
	if err != nil {
		t.Fatalf("Pick() error = %v", err)
	}
	if got == nil {
		t.Fatal("Pick() returned nil")
	}
	return got
}

func TestQuotaHighFirstSelector(t *testing.T) {
	auths := []*Auth{
		metadataAuth("low", map[string]any{"quota_remain": float64(10)}),
		metadataAuth("high", map[string]any{"quota_remain": float64(500)}),
		metadataAuth("mid", map[string]any{"quota_remain": float64(100)}),
		metadataAuth("unknown", nil),
	}
	got := pickOne(t, NewQuotaHighFirstSelector(), auths)
	if got.ID != "high" {
		t.Fatalf("quota_high_first picked %q, want high", got.ID)
	}
}

func TestQuotaLowFirstSelector(t *testing.T) {
	auths := []*Auth{
		metadataAuth("high", map[string]any{"quota_remain": float64(500)}),
		metadataAuth("low", map[string]any{"quota_remain": float64(10)}),
		metadataAuth("unknown", nil),
	}
	got := pickOne(t, NewQuotaLowFirstSelector(), auths)
	if got.ID != "low" {
		t.Fatalf("quota_low_first picked %q, want low", got.ID)
	}
}

func TestPlanHighFirstSelector(t *testing.T) {
	auths := []*Auth{
		metadataAuth("free", map[string]any{"plan_rank": float64(0)}),
		metadataAuth("pro", map[string]any{"plan_rank": float64(2)}),
		metadataAuth("unknown", nil),
	}
	got := pickOne(t, NewPlanHighFirstSelector(), auths)
	if got.ID != "pro" {
		t.Fatalf("plan_high_first picked %q, want pro", got.ID)
	}
}

func TestPlanLowFirstSelector(t *testing.T) {
	auths := []*Auth{
		metadataAuth("pro", map[string]any{"plan_rank": float64(2)}),
		metadataAuth("free", map[string]any{"plan_rank": float64(0)}),
	}
	got := pickOne(t, NewPlanLowFirstSelector(), auths)
	if got.ID != "free" {
		t.Fatalf("plan_low_first picked %q, want free", got.ID)
	}
}

func TestExpirySoonFirstSelector(t *testing.T) {
	auths := []*Auth{
		metadataAuth("later", map[string]any{"subscription_expiry_ms": float64(2_000_000_000_000)}),
		metadataAuth("sooner", map[string]any{"subscription_expiry_ms": float64(1_000_000_000_000)}),
		metadataAuth("no-expiry", map[string]any{"subscription_expiry_ms": float64(0)}),
	}
	got := pickOne(t, NewExpirySoonFirstSelector(), auths)
	if got.ID != "sooner" {
		t.Fatalf("expiry_soon_first picked %q, want sooner", got.ID)
	}
}

func TestQuotaRankStringValue(t *testing.T) {
	auth := metadataAuth("s", map[string]any{"quota_remain": "250"})
	v, ok := authQuotaRemainRank(auth)
	if !ok || v != 250 {
		t.Fatalf("authQuotaRemainRank(string) = %v,%v want 250,true", v, ok)
	}
}

func TestExpiryRankMissingDefaultsFarFuture(t *testing.T) {
	auth := metadataAuth("no-meta", nil)
	v, ok := authExpiryRank(auth)
	if !ok || v == 0 {
		t.Fatalf("authExpiryRank(missing) = %v,%v want far-future,true", v, ok)
	}
}
