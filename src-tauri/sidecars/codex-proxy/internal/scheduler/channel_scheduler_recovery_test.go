package scheduler

import (
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

func TestNextScheduledRecoveryTimeUTC(t *testing.T) {
	tests := []struct {
		name string
		now  time.Time
		want time.Time
	}{
		{
			name: "before midnight slot",
			now:  time.Date(2026, 4, 19, 23, 59, 59, 0, time.UTC),
			want: time.Date(2026, 4, 20, 0, 0, 1, 0, time.UTC),
		},
		{
			name: "between midnight and eight",
			now:  time.Date(2026, 4, 20, 0, 0, 2, 0, time.UTC),
			want: time.Date(2026, 4, 20, 8, 0, 1, 0, time.UTC),
		},
		{
			name: "between eight and sixteen",
			now:  time.Date(2026, 4, 20, 8, 0, 2, 0, time.UTC),
			want: time.Date(2026, 4, 20, 16, 0, 1, 0, time.UTC),
		},
		{
			name: "after sixteen",
			now:  time.Date(2026, 4, 20, 16, 0, 2, 0, time.UTC),
			want: time.Date(2026, 4, 21, 0, 0, 1, 0, time.UTC),
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := NextScheduledRecoveryTimeUTC(tt.now); !got.Equal(tt.want) {
				t.Fatalf("NextScheduledRecoveryTimeUTC() = %s, want %s", got.Format(time.RFC3339), tt.want.Format(time.RFC3339))
			}
		})
	}
}

func TestLastScheduledRecoveryTimeUTC(t *testing.T) {
	tests := []struct {
		name string
		now  time.Time
		want time.Time
	}{
		{
			name: "at midnight slot",
			now:  time.Date(2026, 4, 20, 0, 0, 1, 0, time.UTC),
			want: time.Date(2026, 4, 20, 0, 0, 1, 0, time.UTC),
		},
		{
			name: "before first slot of day",
			now:  time.Date(2026, 4, 20, 0, 0, 0, 0, time.UTC),
			want: time.Date(2026, 4, 19, 16, 0, 1, 0, time.UTC),
		},
		{
			name: "after eight slot",
			now:  time.Date(2026, 4, 20, 8, 30, 0, 0, time.UTC),
			want: time.Date(2026, 4, 20, 8, 0, 1, 0, time.UTC),
		},
		{
			name: "after sixteen slot",
			now:  time.Date(2026, 4, 20, 23, 0, 0, 0, time.UTC),
			want: time.Date(2026, 4, 20, 16, 0, 1, 0, time.UTC),
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := LastScheduledRecoveryTimeUTC(tt.now); !got.Equal(tt.want) {
				t.Fatalf("LastScheduledRecoveryTimeUTC() = %s, want %s", got.Format(time.RFC3339), tt.want.Format(time.RFC3339))
			}
		})
	}
}

func TestMissedScheduledRecoveryTimeUTC(t *testing.T) {
	tests := []struct {
		name        string
		lastChecked time.Time
		now         time.Time
		want        time.Time
		wantOK      bool
	}{
		{
			name:        "no slot crossed",
			lastChecked: time.Date(2026, 4, 20, 7, 0, 0, 0, time.UTC),
			now:         time.Date(2026, 4, 20, 7, 59, 59, 0, time.UTC),
			wantOK:      false,
		},
		{
			name:        "crossed eight slot",
			lastChecked: time.Date(2026, 4, 20, 7, 59, 59, 0, time.UTC),
			now:         time.Date(2026, 4, 20, 8, 19, 0, 0, time.UTC),
			want:        time.Date(2026, 4, 20, 8, 0, 1, 0, time.UTC),
			wantOK:      true,
		},
		{
			name:        "crossed multiple slots returns latest one",
			lastChecked: time.Date(2026, 4, 20, 7, 59, 59, 0, time.UTC),
			now:         time.Date(2026, 4, 20, 16, 30, 0, 0, time.UTC),
			want:        time.Date(2026, 4, 20, 16, 0, 1, 0, time.UTC),
			wantOK:      true,
		},
		{
			name:        "now not after lastChecked",
			lastChecked: time.Date(2026, 4, 20, 8, 0, 2, 0, time.UTC),
			now:         time.Date(2026, 4, 20, 8, 0, 2, 0, time.UTC),
			wantOK:      false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, ok := MissedScheduledRecoveryTimeUTC(tt.lastChecked, tt.now)
			if ok != tt.wantOK {
				t.Fatalf("MissedScheduledRecoveryTimeUTC() ok = %v, want %v", ok, tt.wantOK)
			}
			if tt.wantOK && !got.Equal(tt.want) {
				t.Fatalf("MissedScheduledRecoveryTimeUTC() = %s, want %s", got.Format(time.RFC3339), tt.want.Format(time.RFC3339))
			}
		})
	}
}

func TestRunScheduledRecoveries_UsesMissedSlotTimeSemantics(t *testing.T) {
	missedSlot := time.Date(2026, 4, 20, 8, 0, 1, 0, time.UTC)
	resumeAt := time.Date(2026, 4, 20, 9, 30, 0, 0, time.UTC)
	cfg := config.Config{
		Upstream: []config.UpstreamConfig{{
			Name:        "msg-channel",
			BaseURL:     "https://a.example.com",
			Status:      "suspended",
			APIKeys:     nil,
			ServiceType: "claude",
			DisabledAPIKeys: []config.DisabledKeyInfo{
				{Key: "sk-still-cooling", Reason: "insufficient_balance", DisabledAt: time.Date(2026, 4, 20, 7, 45, 0, 0, time.UTC).Format(time.RFC3339)},
				{Key: "sk-ready", Reason: "insufficient_balance", DisabledAt: time.Date(2026, 4, 20, 6, 30, 0, 0, time.UTC).Format(time.RFC3339)},
			},
		}},
	}

	scheduler, cleanup := createTestScheduler(t, cfg)
	defer cleanup()

	results, err := scheduler.RunScheduledRecoveries(missedSlot)
	if err != nil {
		t.Fatalf("RunScheduledRecoveries() error = %v", err)
	}
	if len(results) != 1 {
		t.Fatalf("results len = %d, want 1", len(results))
	}
	if len(results[0].RestoredKeys) != 1 || results[0].RestoredKeys[0] != "sk-ready" {
		t.Fatalf("RestoredKeys at missed slot = %v, want [sk-ready]", results[0].RestoredKeys)
	}

	updatedAtSlot := scheduler.configManager.GetConfig()
	if len(updatedAtSlot.Upstream[0].DisabledAPIKeys) != 1 || updatedAtSlot.Upstream[0].DisabledAPIKeys[0].Key != "sk-still-cooling" {
		t.Fatalf("DisabledAPIKeys at missed slot = %+v, want only sk-still-cooling left", updatedAtSlot.Upstream[0].DisabledAPIKeys)
	}

	results, err = scheduler.RunScheduledRecoveries(resumeAt)
	if err != nil {
		t.Fatalf("RunScheduledRecoveries() at resume time error = %v", err)
	}
	if len(results) != 1 {
		t.Fatalf("results len at resume time = %d, want 1", len(results))
	}
	if len(results[0].RestoredKeys) != 1 || results[0].RestoredKeys[0] != "sk-still-cooling" {
		t.Fatalf("RestoredKeys at resume time = %v, want [sk-still-cooling]", results[0].RestoredKeys)
	}
}

func TestRunScheduledRecoveries_UsesRecoverAtWhenPresent(t *testing.T) {
	now := time.Date(2026, 4, 20, 8, 0, 1, 0, time.UTC)
	cfg := config.Config{
		Upstream: []config.UpstreamConfig{{
			Name:        "msg-channel",
			BaseURL:     "https://a.example.com",
			Status:      "suspended",
			APIKeys:     nil,
			ServiceType: "claude",
			DisabledAPIKeys: []config.DisabledKeyInfo{
				{Key: "sk-ready", Reason: "insufficient_balance", DisabledAt: now.Add(-30 * time.Minute).Format(time.RFC3339), RecoverAt: now.Add(-time.Minute).Format(time.RFC3339)},
				{Key: "sk-wait", Reason: "insufficient_balance", DisabledAt: now.Add(-3 * time.Hour).Format(time.RFC3339), RecoverAt: now.Add(time.Hour).Format(time.RFC3339)},
			},
		}},
	}

	scheduler, cleanup := createTestScheduler(t, cfg)
	defer cleanup()

	results, err := scheduler.RunScheduledRecoveries(now)
	if err != nil {
		t.Fatalf("RunScheduledRecoveries() error = %v", err)
	}
	if len(results) != 1 {
		t.Fatalf("results len = %d, want 1", len(results))
	}
	if len(results[0].RestoredKeys) != 1 || results[0].RestoredKeys[0] != "sk-ready" {
		t.Fatalf("RestoredKeys = %v, want [sk-ready]", results[0].RestoredKeys)
	}

	updated := scheduler.configManager.GetConfig()
	if len(updated.Upstream[0].DisabledAPIKeys) != 1 || updated.Upstream[0].DisabledAPIKeys[0].Key != "sk-wait" {
		t.Fatalf("DisabledAPIKeys = %+v, want only sk-wait left", updated.Upstream[0].DisabledAPIKeys)
	}
	if len(updated.Upstream[0].APIKeys) != 1 || updated.Upstream[0].APIKeys[0] != "sk-ready" {
		t.Fatalf("APIKeys = %v, want [sk-ready]", updated.Upstream[0].APIKeys)
	}
}
