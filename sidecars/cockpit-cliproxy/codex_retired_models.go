package main

import "strings"

// GPT-5.5 remains valid for explicitly configured API-key providers, but is no
// longer available to accounts signed in with ChatGPT.
func isRetiredChatGPTModel(m *manifest, model string) bool {
	model = strings.ToLower(strings.TrimSpace(model))
	if m != nil {
		for _, alias := range m.ModelAliases {
			if strings.EqualFold(strings.TrimSpace(alias.Alias), model) {
				model = strings.ToLower(strings.TrimSpace(alias.SourceModel))
				break
			}
		}
	}
	return model == "gpt-5.5" || strings.HasPrefix(model, "gpt-5.5-") || strings.HasPrefix(model, "gpt-5.5(")
}

func activeChatGPTModels(m *manifest, models []string) []string {
	active := make([]string, 0, len(models))
	for _, model := range models {
		if !isRetiredChatGPTModel(m, model) {
			active = append(active, model)
		}
	}
	return active
}
