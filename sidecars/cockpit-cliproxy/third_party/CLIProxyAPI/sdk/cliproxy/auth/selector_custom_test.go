package auth

import (
	"context"
	"testing"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func customAuth(id string, priority float64, weight float64, backup, preferred bool) *Auth {
	return &Auth{
		ID:       id,
		Provider: "codebuddy",
		Metadata: map[string]any{
			"routing_priority":   priority,
			"routing_weight":     weight,
			"routing_is_backup":  backup,
			"routing_is_preferred": preferred,
		},
	}
}

func TestCustomSelectorPreferredFirst(t *testing.T) {
	auths := []*Auth{
		customAuth("normal", 5, 1, false, false),
		customAuth("preferred", 1, 1, false, true),
		customAuth("backup", 10, 1, true, false),
	}
	sel := NewCustomSelector()
	got, err := sel.Pick(context.Background(), "codebuddy", "deepseek-v4-flash",
		cliproxyexecutor.Options{}, auths)
	if err != nil {
		t.Fatalf("Pick() error = %v", err)
	}
	if got.ID != "preferred" {
		t.Fatalf("custom selector picked %q, want preferred", got.ID)
	}
}

func TestCustomSelectorPriorityOrdering(t *testing.T) {
	auths := []*Auth{
		customAuth("low", 1, 1, false, false),
		customAuth("high", 10, 1, false, false),
		customAuth("mid", 5, 1, false, false),
	}
	sel := NewCustomSelector()
	got, err := sel.Pick(context.Background(), "codebuddy", "deepseek-v4-flash",
		cliproxyexecutor.Options{}, auths)
	if err != nil {
		t.Fatalf("Pick() error = %v", err)
	}
	if got.ID != "high" {
		t.Fatalf("custom selector picked %q, want high", got.ID)
	}
}

func TestCustomSelectorBackupLast(t *testing.T) {
	auths := []*Auth{
		customAuth("backup", 100, 1, true, false),
		customAuth("normal", 0, 1, false, false),
	}
	sel := NewCustomSelector()
	got, err := sel.Pick(context.Background(), "codebuddy", "deepseek-v4-flash",
		cliproxyexecutor.Options{}, auths)
	if err != nil {
		t.Fatalf("Pick() error = %v", err)
	}
	if got.ID != "normal" {
		t.Fatalf("custom selector picked %q, want normal (backup should be last)", got.ID)
	}
}

func TestCustomSelectorWeightedRoundRobin(t *testing.T) {
	// Two equal-priority accounts, A with weight 3 and B with weight 1.
	// Over 4 picks, A should be picked more often than B.
	auths := []*Auth{
		customAuth("a", 0, 3, false, false),
		customAuth("b", 0, 1, false, false),
	}
	sel := NewCustomSelector()
	counts := map[string]int{}
	for i := 0; i < 4; i++ {
		got, err := sel.Pick(context.Background(), "codebuddy", "deepseek-v4-flash",
			cliproxyexecutor.Options{}, auths)
		if err != nil {
			t.Fatalf("Pick() error = %v", err)
		}
		counts[got.ID]++
	}
	if counts["a"] != 3 || counts["b"] != 1 {
		t.Fatalf("weighted round-robin distribution = %v, want a:3 b:1", counts)
	}
}

func TestAuthRoutingDefaults(t *testing.T) {
	auth := metadataAuth("plain", nil)
	if authRoutingPriority(auth) != 0 {
		t.Fatal("default priority should be 0")
	}
	if authRoutingWeight(auth) != 1 {
		t.Fatal("default weight should be 1")
	}
	if authRoutingIsBackup(auth) || authRoutingIsPreferred(auth) {
		t.Fatal("default flags should be false")
	}
}
