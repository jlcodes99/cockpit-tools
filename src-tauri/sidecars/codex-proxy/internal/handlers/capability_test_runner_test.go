package handlers

import "testing"

func TestShouldRunRedirectVerification(t *testing.T) {
	tests := []struct {
		name               string
		protocols          []string
		sourceTab          string
		channelServiceType string
		want               bool
	}{
		{
			name:               "single unrelated protocol does not run redirect verification",
			protocols:          []string{"messages"},
			sourceTab:          "responses",
			channelServiceType: "chat",
			want:               false,
		},
		{
			name:               "source protocol does not run redirect verification implicitly",
			protocols:          []string{"responses"},
			sourceTab:          "responses",
			channelServiceType: "chat",
			want:               false,
		},
		{
			name:               "explicit virtual protocol runs redirect verification",
			protocols:          []string{"responses->chat"},
			sourceTab:          "responses",
			channelServiceType: "chat",
			want:               true,
		},
		{
			name:               "other virtual protocol does not run redirect verification",
			protocols:          []string{"messages->chat"},
			sourceTab:          "responses",
			channelServiceType: "chat",
			want:               false,
		},
		{
			name:               "all base protocols do not run redirect verification implicitly",
			protocols:          []string{"messages", "responses", "chat", "gemini"},
			sourceTab:          "responses",
			channelServiceType: "chat",
			want:               false,
		},
		{
			name:               "same source and channel protocol does not run redirect verification implicitly",
			protocols:          []string{"responses"},
			sourceTab:          "responses",
			channelServiceType: "responses",
			want:               false,
		},
		{
			name:               "same source explicit virtual protocol runs redirect verification",
			protocols:          []string{"responses->responses"},
			sourceTab:          "responses",
			channelServiceType: "responses",
			want:               true,
		},
		{
			name:               "empty source tab does not run redirect verification",
			protocols:          []string{"responses"},
			sourceTab:          "",
			channelServiceType: "chat",
			want:               false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := shouldRunRedirectVerification(tt.protocols, tt.sourceTab, tt.channelServiceType)
			if got != tt.want {
				t.Fatalf("shouldRunRedirectVerification(%v, %q, %q) = %v, want %v", tt.protocols, tt.sourceTab, tt.channelServiceType, got, tt.want)
			}
		})
	}
}
