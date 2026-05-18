package config

import "testing"

func TestNormalizeUILanguage(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name  string
		input string
		want  string
	}{
		{name: "english", input: "en", want: "en"},
		{name: "indonesian", input: "id", want: "id"},
		{name: "chinese short", input: "zh", want: "zh-CN"},
		{name: "chinese full", input: "zh-CN", want: "zh-CN"},
		{name: "invalid falls back", input: "fr", want: "en"},
		{name: "empty falls back", input: "", want: "en"},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()
			if got := normalizeUILanguage(tt.input); got != tt.want {
				t.Fatalf("normalizeUILanguage(%q) = %q, want %q", tt.input, got, tt.want)
			}
		})
	}
}
