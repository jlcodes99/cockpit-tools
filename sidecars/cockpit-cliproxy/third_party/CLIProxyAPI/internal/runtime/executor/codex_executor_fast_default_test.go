package executor

import (
	"bytes"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/translator"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestCodexFastDefaultRespectsExplicitClientTierAfterTranslation(t *testing.T) {
	cfg := &config.Config{Payload: config.PayloadConfig{Default: []config.PayloadRule{{
		Models: []config.PayloadModelRule{{Name: "*", Protocol: "codex"}},
		Params: map[string]any{"service_tier": "priority"},
	}}}}
	for _, from := range []sdktranslator.Format{sdktranslator.FormatOpenAIResponse, sdktranslator.FormatOpenAI} {
		for _, tier := range []string{"", "default", "standard", "flex", "auto", "fast", "priority"} {
			for _, same := range []bool{true, false} {
				t.Run(string(from)+"/"+tier+"/"+map[bool]string{true: "same", false: "different"}[same], func(t *testing.T) {
					base := `{"model":"custom-model","input":"fixture","messages":[{"role":"user","content":"fixture"}]`
					if tier != "" {
						base += `,"service_tier":"` + tier + `"`
					}
					original := []byte(base + `}`)
					payload := bytes.Clone(original)
					if !same {
						payload = []byte(base + `,"stream":true}`)
					}
					snapshot, body := translateCodexRequestPair(from, sdktranslator.FormatCodex, "custom-model", original, payload, true)
					forwarded := helps.ApplyPayloadConfigWithRequest(cfg, "custom-model", "codex", string(from), "", body, snapshot, "custom-model", "/v1/responses", nil)
					want := ""
					if tier == "" || tier == "fast" || tier == "priority" {
						want = "priority"
					}
					if got := gjson.GetBytes(forwarded, "service_tier").String(); got != want {
						t.Fatalf("tier %q: forwarded %q, want %q", tier, got, want)
					}
					if tier == "default" || tier == "standard" || tier == "flex" || tier == "auto" {
						if gjson.GetBytes(body, "service_tier").Exists() {
							t.Fatal("defaults snapshot must not mutate the OAuth upstream body")
						}
					}
				})
			}
		}
	}
}
