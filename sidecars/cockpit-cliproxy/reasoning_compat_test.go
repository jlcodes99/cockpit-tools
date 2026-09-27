package main

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestApplyDeepSeekReasoningDefaults(t *testing.T) {
	configPath := filepath.Join(t.TempDir(), "config.json")
	raw := `{
		"codex-api-key": [{
			"api-key": "test-key",
			"base-url": "https://example.com/v1",
			"models": [
				{"name": "deepseek-flash", "alias": "deepseek-flash"},
				{"name": "gpt-5.4", "alias": "gpt-5.4"},
				{"name": "deepseek-v4-pro", "alias": "deepseek-v4-pro", "thinking": {"levels": ["high"]}}
			]
		}]
	}`
	if err := os.WriteFile(configPath, []byte(raw), 0o600); err != nil {
		t.Fatalf("write config: %v", err)
	}
	cfg, err := config.LoadConfig(configPath)
	if err != nil {
		t.Fatalf("LoadConfig() error = %v", err)
	}

	applyDeepSeekReasoningDefaults(cfg)

	models := cfg.CodexKey[0].Models
	if models[0].Thinking == nil || !reflect.DeepEqual(models[0].Thinking.Levels, []string{"max", "low", "medium", "high"}) {
		t.Fatalf("deepseek-flash thinking = %#v, want max-first levels", models[0].Thinking)
	}
	if models[1].Thinking != nil {
		t.Fatalf("non-DeepSeek model thinking = %#v, want nil", models[1].Thinking)
	}
	if models[2].Thinking == nil || !reflect.DeepEqual(models[2].Thinking.Levels, []string{"high"}) {
		t.Fatalf("explicit thinking = %#v, want preserved", models[2].Thinking)
	}
}
