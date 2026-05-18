package config

import "testing"

func codexCompatBoolPtr(b bool) *bool {
	return &b
}

func TestCodexToolCompatPriority(t *testing.T) {
	tests := []struct {
		name                  string
		codexToolCompat       *bool
		stripCodexClientTools bool
		want                  bool
	}{
		{
			name:            "codexToolCompat true overrides default",
			codexToolCompat: codexCompatBoolPtr(true),
			want:            true,
		},
		{
			name:                  "codexToolCompat false overrides legacy true",
			codexToolCompat:       codexCompatBoolPtr(false),
			stripCodexClientTools: true,
			want:                  false,
		},
		{
			name:                  "legacy stripCodexClientTools true fallback",
			stripCodexClientTools: true,
			want:                  true,
		},
		{
			name: "both unset defaults to false",
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			up := &UpstreamConfig{
				CodexToolCompat:       tt.codexToolCompat,
				StripCodexClientTools: tt.stripCodexClientTools,
			}
			if got := up.IsCodexToolCompatEnabled(); got != tt.want {
				t.Errorf("IsCodexToolCompatEnabled() = %v, want %v", got, tt.want)
			}
		})
	}
}
