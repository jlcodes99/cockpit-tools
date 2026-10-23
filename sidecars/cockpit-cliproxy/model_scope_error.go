package main

import "strings"

// Model access failures share the host's locale across native, provider, image,
// and search routes. Keep a useful fallback for standalone/test manifests.
func (m *manifest) modelUnavailableMessage(model string) string {
	message := "Model {{model}} is not available for this API key."
	if m != nil {
		locale := strings.ToLower(strings.TrimSpace(m.Locale))
		translations := m.GatewayErrorMessages["model_not_available"]
		for _, candidate := range []string{locale, strings.Split(locale, "-")[0], "en"} {
			for language, value := range translations {
				if strings.EqualFold(language, candidate) && strings.TrimSpace(value) != "" {
					return strings.ReplaceAll(strings.TrimSpace(value), "{{model}}", model)
				}
			}
		}
	}
	return strings.ReplaceAll(message, "{{model}}", model)
}
