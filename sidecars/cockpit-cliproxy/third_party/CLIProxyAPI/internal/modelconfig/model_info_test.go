package modelconfig

import (
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/thinking"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/thinking/provider/codex"
	"github.com/tidwall/gjson"
)

func TestConfiguredGPT6ModelsPreserveResponsesLiteReasoning(t *testing.T) {
	for _, model := range []string{"gpt-6.1-sol", "gpt-6-astra", "gpt-6-sol", "gpt-6-luna"} {
		info := ResolveModelInfo(model, "codex", nil)
		if info.Thinking == nil {
			t.Fatalf("%s has no reasoning capabilities", model)
		}
		body := []byte(`{"reasoning":{"effort":"high","context":"all_turns"}}`)
		out, err := thinking.ApplyThinkingWithModelInfo(body, body, model, "openai-response", "codex", "codex", info)
		if err != nil {
			t.Fatalf("%s: %v", model, err)
		}
		if gjson.GetBytes(out, "reasoning.context").String() != "all_turns" || gjson.GetBytes(out, "reasoning.effort").String() != "high" {
			t.Fatalf("%s lost Responses Lite reasoning: %s", model, out)
		}
	}
}

func TestResolveModelInfoUsesSuffixFreeStaticCapabilities(t *testing.T) {
	info := ResolveModelInfo("claude-opus-4-6(high)", "claude", nil)
	if info == nil || info.Thinking == nil {
		t.Fatalf("ResolveModelInfo() = %+v, want inherited thinking support", info)
	}
	if info.ID != "claude-opus-4-6(high)" {
		t.Fatalf("model ID = %q, want configured upstream name", info.ID)
	}
	if info.UserDefined {
		t.Fatal("resolved capability snapshot must not be user-defined")
	}
}

func TestResolveModelInfoExplicitThinkingOverridesAndClones(t *testing.T) {
	support := &registry.ThinkingSupport{Levels: []string{" XHIGH ", "xhigh", " High "}}
	info := ResolveModelInfo("custom-model", "codex", support)
	if info == nil || info.Thinking == nil {
		t.Fatalf("ResolveModelInfo() = %+v, want explicit thinking support", info)
	}
	if got := info.Thinking.Levels; len(got) != 2 || got[0] != "xhigh" || got[1] != "high" {
		t.Fatalf("normalized levels = %v, want [xhigh high]", got)
	}
	support.Levels[0] = "low"
	if info.Thinking.Levels[0] != "xhigh" {
		t.Fatal("resolved thinking support shares mutable config storage")
	}
}

func TestNormalizeThinkingSupportDerivesSpecialLevelFlags(t *testing.T) {
	support := NormalizeThinkingSupport(&registry.ThinkingSupport{Levels: []string{"low", "none", "auto"}})
	if support == nil {
		t.Fatal("NormalizeThinkingSupport() = nil")
	}
	if !support.ZeroAllowed {
		t.Fatal("none level did not enable ZeroAllowed")
	}
	if !support.DynamicAllowed {
		t.Fatal("auto level did not enable DynamicAllowed")
	}
}

func TestResolveModelInfoUnknownModelKeepsMissingCapability(t *testing.T) {
	info := ResolveModelInfo("unknown-configured-model", "claude", nil)
	if info == nil {
		t.Fatal("ResolveModelInfo() = nil")
	}
	if info.Thinking != nil {
		t.Fatalf("unknown model thinking = %+v, want nil", info.Thinking)
	}
	if info.UserDefined {
		t.Fatal("unknown configured model must use its exact bound capability")
	}
}
