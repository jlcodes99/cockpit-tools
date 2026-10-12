package executor

import (
	"context"
	"sync/atomic"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

func TestCodexOpenAIImageServiceTierDefaultLeavesImageSettings(t *testing.T) {
	cfg := &config.Config{Payload: config.PayloadConfig{Default: []config.PayloadRule{{Models: []config.PayloadModelRule{{Name: "*", Protocol: "codex"}}, Params: map[string]any{"service_tier": "priority"}}}}}
	cfg.DisableImageGeneration = config.DisableImageGenerationOff
	exec := NewCodexExecutor(cfg)
	req := coreexecutor.Request{Model: "gpt-image-2"}
	opts := coreexecutor.Options{Metadata: map[string]any{coreexecutor.RequestPathMetadataKey: "/v1/images/generations"}}
	original := []byte(`{"model":"gpt-5.5","input":[{"role":"user","content":"fixture"}],"tools":[{"type":"image_generation","model":"gpt-image-2","size":"1024x1024","quality":"high"}]}`)
	var current atomic.Value
	current.Store("")
	parent := coreexecutor.WithServiceTierDefaultProvider(context.Background(), func() string { return current.Load().(string) })
	for _, tier := range []string{"", "priority", ""} {
		current.Store(tier)
		out, err := exec.prepareCodexOpenAIImageBody(original, req, opts, "gpt-5.5", coreexecutor.SnapshotServiceTierDefault(parent))
		if err != nil {
			t.Fatal(err)
		}
		if got := gjson.GetBytes(out, "service_tier").String(); got != tier {
			t.Fatalf("tier=%q want=%q", got, tier)
		}
		for _, path := range []string{"model", "tools", "input"} {
			if got, want := gjson.GetBytes(out, path).Raw, gjson.GetBytes(original, path).Raw; got != want {
				t.Fatalf("%s changed: got=%s want=%s", path, got, want)
			}
		}
	}
	out, err := exec.prepareCodexOpenAIImageBody(original, req, opts, "gpt-5.5")
	if err != nil {
		t.Fatal(err)
	}
	if got := gjson.GetBytes(out, "service_tier").String(); got != "priority" {
		t.Fatalf("unconfigured host default changed: %q", got)
	}
	explicit := []byte(`{"service_tier":"flex","input":[],"tools":[{"type":"image_generation","model":"gpt-image-2"}]}`)
	current.Store("priority")
	out, err = exec.prepareCodexOpenAIImageBody(explicit, req, opts, "gpt-5.5", coreexecutor.SnapshotServiceTierDefault(parent))
	if err != nil {
		t.Fatal(err)
	}
	if got := gjson.GetBytes(out, "service_tier").String(); got != "flex" {
		t.Fatalf("explicit tier changed: %q", got)
	}
}
