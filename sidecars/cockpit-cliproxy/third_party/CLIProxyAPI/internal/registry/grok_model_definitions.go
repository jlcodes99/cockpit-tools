package registry

// Pin the released model's capabilities even when a remote catalog is older.
// https://docs.x.ai/developers/models/grok-4.7
func xaiBuiltinGrok47ModelInfo() *ModelInfo {
	return &ModelInfo{
		ID: "grok-4.7", Object: "model", Created: 1789948800,
		OwnedBy: "xai", Type: "xai", DisplayName: "Grok 4.7", Name: "grok-4.7",
		Description: "Grok 4.7", ContextLength: 500000, MaxCompletionTokens: 500000,
		SupportedInputModalities:  []string{"text", "image"},
		SupportedOutputModalities: []string{"text"},
		Thinking:                  &ThinkingSupport{Levels: []string{"low", "medium", "high", "xhigh"}, DefaultLevel: "high"},
	}
}
