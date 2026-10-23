package kimi

import (
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/thinking"
	"github.com/tidwall/gjson"
)

func TestApply_ModeNone_UsesDisabledThinking(t *testing.T) {
	applier := NewApplier()
	modelInfo := &registry.ModelInfo{
		ID:       "kimi-k2.5",
		Thinking: &registry.ThinkingSupport{Min: 1024, Max: 32000, ZeroAllowed: true, DynamicAllowed: true},
	}
	body := []byte(`{"model":"kimi-k2.5","reasoning_effort":"none","thinking":{"type":"enabled","budget_tokens":2048}}`)

	out, errApply := applier.Apply(body, thinking.ThinkingConfig{Mode: thinking.ModeNone}, modelInfo)
	if errApply != nil {
		t.Fatalf("Apply() error = %v", errApply)
	}
	if got := gjson.GetBytes(out, "thinking.type").String(); got != "disabled" {
		t.Fatalf("thinking.type = %q, want %q, body=%s", got, "disabled", string(out))
	}
	if gjson.GetBytes(out, "thinking.budget_tokens").Exists() {
		t.Fatalf("thinking.budget_tokens should be removed, body=%s", string(out))
	}
	if gjson.GetBytes(out, "reasoning_effort").Exists() {
		t.Fatalf("reasoning_effort should be removed in ModeNone, body=%s", string(out))
	}
}

func TestApply_ModeLevel_UsesNativeThinkingAndPreservesKeep(t *testing.T) {
	applier := NewApplier()
	modelInfo := &registry.ModelInfo{
		ID:       "kimi-k2.5",
		Thinking: &registry.ThinkingSupport{Min: 1024, Max: 32000, ZeroAllowed: true, DynamicAllowed: true},
	}
	body := []byte(`{"model":"kimi-k2.5","reasoning_effort":"low","thinking":{"type":"disabled","keep":"all"}}`)

	out, errApply := applier.Apply(body, thinking.ThinkingConfig{Mode: thinking.ModeLevel, Level: thinking.LevelHigh}, modelInfo)
	if errApply != nil {
		t.Fatalf("Apply() error = %v", errApply)
	}
	if got := gjson.GetBytes(out, "thinking.type").String(); got != "enabled" {
		t.Fatalf("thinking.type = %q, want enabled", got)
	}
	if got := gjson.GetBytes(out, "thinking.effort").String(); got != "high" {
		t.Fatalf("thinking.effort = %q, want high", got)
	}
	if got := gjson.GetBytes(out, "thinking.keep").String(); got != "all" {
		t.Fatalf("thinking.keep = %q, want all", got)
	}
	if gjson.GetBytes(out, "reasoning_effort").Exists() {
		t.Fatalf("legacy reasoning_effort should be removed, body=%s", out)
	}
}

func TestApply_UserDefinedModeNone_UsesDisabledThinking(t *testing.T) {
	applier := NewApplier()
	modelInfo := &registry.ModelInfo{
		ID:          "custom-kimi-model",
		UserDefined: true,
	}
	body := []byte(`{"model":"custom-kimi-model","reasoning_effort":"none"}`)

	out, errApply := applier.Apply(body, thinking.ThinkingConfig{Mode: thinking.ModeNone}, modelInfo)
	if errApply != nil {
		t.Fatalf("Apply() error = %v", errApply)
	}
	if got := gjson.GetBytes(out, "thinking.type").String(); got != "disabled" {
		t.Fatalf("thinking.type = %q, want %q, body=%s", got, "disabled", string(out))
	}
	if gjson.GetBytes(out, "reasoning_effort").Exists() {
		t.Fatalf("reasoning_effort should be removed in ModeNone, body=%s", string(out))
	}
}
