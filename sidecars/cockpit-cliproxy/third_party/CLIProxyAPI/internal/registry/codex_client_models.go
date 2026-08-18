package registry

import (
	_ "embed"
	"encoding/json"
	"strings"
	"sync"
)

//go:embed models/codex_client_models.json
var codexClientModelsJSON []byte

type codexClientModelOverridesPayload struct {
	ModelOverrides []codexClientModelOverride `json:"model_overrides"`
	Models         []codexClientModelOverride `json:"models"`
}

type codexClientModelOverride struct {
	Slug                     string                      `json:"slug"`
	DisplayName              string                      `json:"display_name"`
	Description              string                      `json:"description"`
	BaseInstructions         string                      `json:"base_instructions"`
	ContextWindow            int                         `json:"context_window"`
	UseResponsesLite         bool                        `json:"use_responses_lite"`
	SupportedReasoningLevels []codexClientReasoningLevel `json:"supported_reasoning_levels"`
}

type codexClientReasoningLevel struct {
	Effort string `json:"effort"`
}

var (
	codexClientBuiltinModelsOnce sync.Once
	codexClientBuiltinModels     []*ModelInfo
	codexResponsesLiteModels     map[string]struct{}
	codexClientBaseInstructions  map[string]string
)

const codexClientFallbackInstructions = "You are Codex, a coding agent. Help the user complete software engineering tasks accurately and safely."

// GetCodexClientModelsJSON returns the embedded Codex client model catalog.
func GetCodexClientModelsJSON() []byte {
	return append([]byte(nil), codexClientModelsJSON...)
}

func codexClientBuiltinModelInfos() []*ModelInfo {
	codexClientBuiltinModelsOnce.Do(func() {
		var payload codexClientModelOverridesPayload
		if err := json.Unmarshal(codexClientModelsJSON, &payload); err != nil {
			return
		}
		codexResponsesLiteModels = make(map[string]struct{})
		codexClientBaseInstructions = make(map[string]string)
		seen := make(map[string]struct{})

		register := func(model codexClientModelOverride) {
			slug := strings.TrimSpace(model.Slug)
			if slug == "" {
				return
			}
			if model.UseResponsesLite {
				codexResponsesLiteModels[strings.ToLower(slug)] = struct{}{}
			}
			if instructions := strings.TrimSpace(model.BaseInstructions); instructions != "" {
				codexClientBaseInstructions[strings.ToLower(slug)] = model.BaseInstructions
			}
			if _, ok := seen[slug]; ok {
				return
			}
			// Prefer override entries for builtin ModelInfo (richer display/reasoning);
			// full models also contribute when overrides are absent.
			levels := make([]string, 0, len(model.SupportedReasoningLevels))
			for _, rawLevel := range model.SupportedReasoningLevels {
				level := strings.ToLower(strings.TrimSpace(rawLevel.Effort))
				if level != "" {
					levels = append(levels, level)
				}
			}
			// Only materialize ModelInfo when we have usable metadata.
			if model.DisplayName == "" && model.ContextWindow == 0 && len(levels) == 0 && !model.UseResponsesLite {
				return
			}
			seen[slug] = struct{}{}
			var thinking *ThinkingSupport
			if len(levels) > 0 {
				thinking = &ThinkingSupport{Levels: levels}
			}
			codexClientBuiltinModels = append(codexClientBuiltinModels, &ModelInfo{
				ID:            slug,
				Object:        "model",
				OwnedBy:       "openai",
				Type:          "openai",
				DisplayName:   model.DisplayName,
				Version:       slug,
				Description:   model.Description,
				ContextLength: model.ContextWindow,
				Thinking:      thinking,
			})
		}

		// Overrides first so ModelInfo prefers the compact metadata block.
		for _, model := range payload.ModelOverrides {
			register(model)
		}
		for _, model := range payload.Models {
			register(model)
		}
	})

	return cloneModelInfos(codexClientBuiltinModels)
}

// CodexClientModelUsesResponsesLite reports whether the embedded Codex client
// catalog routes the model through the Responses Lite protocol.
func CodexClientModelUsesResponsesLite(modelID string) bool {
	codexClientBuiltinModelInfos()
	_, ok := codexResponsesLiteModels[strings.ToLower(strings.TrimSpace(modelID))]
	return ok
}

// CodexClientModelBaseInstructions returns the embedded Codex instructions for
// a model. Unknown models use the current GPT-5.5 instructions when available.
func CodexClientModelBaseInstructions(modelID string) string {
	codexClientBuiltinModelInfos()
	if instructions := codexClientBaseInstructions[strings.ToLower(strings.TrimSpace(modelID))]; strings.TrimSpace(instructions) != "" {
		return instructions
	}
	if instructions := codexClientBaseInstructions["gpt-5.5"]; strings.TrimSpace(instructions) != "" {
		return instructions
	}
	return codexClientFallbackInstructions
}
