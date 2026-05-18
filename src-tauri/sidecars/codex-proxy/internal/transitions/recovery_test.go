package transitions

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
)

func TestRestoreDisabledKeysAndActivate(t *testing.T) {
	cfg := config.Config{Upstream: []config.UpstreamConfig{{
		Name:        "msg-channel",
		BaseURL:     "https://example.com",
		Status:      "suspended",
		APIKeys:     nil,
		ServiceType: "claude",
		DisabledAPIKeys: []config.DisabledKeyInfo{{
			Key: "sk-ready", Reason: "insufficient_balance", DisabledAt: time.Now().Add(-2 * time.Hour).Format(time.RFC3339), RecoverAt: time.Now().Add(-time.Minute).Format(time.RFC3339),
		}},
	}}}

	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		t.Fatalf("marshal config: %v", err)
	}
	configDir := t.TempDir()
	configFile := filepath.Join(configDir, "config.json")
	if err := os.WriteFile(configFile, data, 0644); err != nil {
		t.Fatalf("write config: %v", err)
	}

	cfgManager, err := config.NewConfigManager(configFile)
	if err != nil {
		t.Fatalf("new config manager: %v", err)
	}
	defer cfgManager.Close()

	metricsManager := metrics.NewMetricsManager()
	defer metricsManager.Stop()

	activated := false
	result, err := RestoreDisabledKeysAndActivate(
		func(keys []string) ([]string, error) {
			return cfgManager.RestoreDisabledKeys("Messages", 0, keys)
		},
		func(_ string, apiKey string) {
			metricsManager.MoveKeyToHalfOpen("https://example.com/v1", apiKey, "claude")
		},
		func(status string) error {
			activated = true
			return cfgManager.SetChannelStatus(0, status)
		},
		func() bool {
			latest := cfgManager.GetConfig().Upstream[0]
			return latest.Status == "suspended"
		},
		[]string{"sk-ready"},
	)
	if err != nil {
		t.Fatalf("RestoreDisabledKeysAndActivate() error = %v", err)
	}
	if !activated || !result.ActivatedChannel {
		t.Fatalf("ActivatedChannel = %v/%v, want true/true", activated, result.ActivatedChannel)
	}
	if len(result.RestoredKeys) != 1 || result.RestoredKeys[0] != "sk-ready" {
		t.Fatalf("RestoredKeys = %v, want [sk-ready]", result.RestoredKeys)
	}
	if got := metricsManager.GetKeyCircuitState("https://example.com/v1", "sk-ready", "claude"); got != metrics.CircuitStateHalfOpen {
		t.Fatalf("circuit state = %v, want half_open", got)
	}
}

func TestRestoreDisabledKeysAndActivate_DoesNotOverrideDisabledChannel(t *testing.T) {
	activated := false
	result, err := RestoreDisabledKeysAndActivate(
		func(keys []string) ([]string, error) { return keys, nil },
		func(_ string, _ string) {},
		func(string) error {
			activated = true
			return nil
		},
		func() bool { return false },
		[]string{"sk-ready"},
	)
	if err != nil {
		t.Fatalf("RestoreDisabledKeysAndActivate() error = %v", err)
	}
	if activated || result.ActivatedChannel {
		t.Fatalf("ActivatedChannel = %v/%v, want false/false", activated, result.ActivatedChannel)
	}
}
