package config

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLegacyAmpCompatibilityDoesNotRestoreProductConfig(t *testing.T) {
	path := filepath.Join(t.TempDir(), "config.yaml")
	input := "# preserved\nport: 8080\nampcode:\n  upstream-url: https://legacy.example\n  upstream-api-key: legacy-secret\namp-upstream-url: https://older.example\n"
	if err := os.WriteFile(path, []byte(input), 0600); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.AmpCode.UpstreamURL != "https://legacy.example" {
		t.Fatal("legacy packages could not read their compatibility DTO")
	}
	productJSON, err := json.Marshal(cfg)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(productJSON), "ampcode") || strings.Contains(string(productJSON), "legacy-secret") {
		t.Fatal("legacy Amp configuration leaked into product JSON")
	}
	if err := SaveConfigPreserveComments(path, cfg); err != nil {
		t.Fatal(err)
	}
	productYAML, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(productYAML), "ampcode") || strings.Contains(string(productYAML), "amp-upstream-url") || strings.Contains(string(productYAML), "legacy-secret") {
		t.Fatalf("product save restored a removed integration: %s", productYAML)
	}
	if !strings.Contains(string(productYAML), "# preserved") || !strings.Contains(string(productYAML), "port: 8080") {
		t.Fatal("product config lost unrelated values or comments")
	}
}
