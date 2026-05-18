package main

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
)

func TestLoadScheduledRecoveryLastCheck_MissingFile(t *testing.T) {
	got, err := loadScheduledRecoveryLastCheck(filepath.Join(t.TempDir(), "missing.json"))
	if err != nil {
		t.Fatalf("loadScheduledRecoveryLastCheck() error = %v", err)
	}
	if !got.IsZero() {
		t.Fatalf("loadScheduledRecoveryLastCheck() = %s, want zero time", got.Format(time.RFC3339Nano))
	}
}

func TestMissingRecoveryStateRequiresCallerGuard(t *testing.T) {
	lastChecked, err := loadScheduledRecoveryLastCheck(filepath.Join(t.TempDir(), "missing.json"))
	if err != nil {
		t.Fatalf("loadScheduledRecoveryLastCheck() error = %v", err)
	}
	if !lastChecked.IsZero() {
		t.Fatalf("lastChecked = %s, want zero time", lastChecked.Format(time.RFC3339Nano))
	}
	if !lastChecked.IsZero() {
		t.Fatal("missing recovery state should stay zero so caller can skip startup catch-up")
	}
	if missedSlot, ok := scheduler.MissedScheduledRecoveryTimeUTC(time.Date(2026, 4, 22, 7, 59, 59, 0, time.UTC), time.Date(2026, 4, 22, 8, 19, 0, 0, time.UTC)); !ok || missedSlot.IsZero() {
		t.Fatal("control case: non-zero last check should still detect missed slots")
	}
}

func TestShouldCommitRecoveryCheck(t *testing.T) {
	tests := []struct {
		name      string
		attempted bool
		succeeded bool
		want      bool
	}{
		{name: "no attempt commits", attempted: false, succeeded: true, want: true},
		{name: "successful attempt commits", attempted: true, succeeded: true, want: true},
		{name: "failed attempt keeps checkpoint", attempted: true, succeeded: false, want: false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := shouldCommitRecoveryCheck(tt.attempted, tt.succeeded)
			if got != tt.want {
				t.Fatalf("shouldCommitRecoveryCheck(%v, %v) = %v, want %v", tt.attempted, tt.succeeded, got, tt.want)
			}
		})
	}
}

func TestSaveScheduledRecoveryLastCheck_RoundTrip(t *testing.T) {
	path := filepath.Join(t.TempDir(), "scheduled_recovery_state.json")
	want := time.Date(2026, 4, 22, 8, 19, 0, 123456789, time.FixedZone("CST", 8*3600))

	if err := saveScheduledRecoveryLastCheck(path, want); err != nil {
		t.Fatalf("saveScheduledRecoveryLastCheck() error = %v", err)
	}

	got, err := loadScheduledRecoveryLastCheck(path)
	if err != nil {
		t.Fatalf("loadScheduledRecoveryLastCheck() error = %v", err)
	}
	if !got.Equal(want.UTC()) {
		t.Fatalf("round trip time = %s, want %s", got.Format(time.RFC3339Nano), want.UTC().Format(time.RFC3339Nano))
	}
}
