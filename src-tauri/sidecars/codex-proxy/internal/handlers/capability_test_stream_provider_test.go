package handlers

import (
	"context"
	"reflect"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers/common"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/providers"
)

func TestGetCapabilityStreamProvider(t *testing.T) {
	tests := []struct {
		protocol string
		wantType any
	}{
		{protocol: "messages", wantType: &providers.ClaudeProvider{}},
		{protocol: "chat", wantType: &providers.OpenAIProvider{}},
		{protocol: "gemini", wantType: &providers.GeminiProvider{}},
		{protocol: "responses", wantType: &providers.ResponsesProvider{}},
	}

	for _, tt := range tests {
		t.Run(tt.protocol, func(t *testing.T) {
			got := getCapabilityStreamProvider(tt.protocol)
			if got == nil {
				t.Fatalf("getCapabilityStreamProvider(%q) returned nil", tt.protocol)
			}
			if reflect.TypeOf(got) != reflect.TypeOf(tt.wantType) {
				t.Fatalf("getCapabilityStreamProvider(%q) type = %T, want %T", tt.protocol, got, tt.wantType)
			}
		})
	}

	if got := getCapabilityStreamProvider("unknown"); got != nil {
		t.Fatalf("getCapabilityStreamProvider(%q) = %T, want nil", "unknown", got)
	}
}

func TestClassifyError_EmptyStreamReturnsSemanticReason(t *testing.T) {
	if got := classifyError(common.ErrEmptyStreamResponse, 0, context.Background()); got != "empty_response" {
		t.Fatalf("classifyError(empty stream) = %q, want %q", got, "empty_response")
	}
}

func TestIsTimedOutPreflightResult(t *testing.T) {
	if !isTimedOutPreflightResult(&common.StreamPreflightResult{}) {
		t.Fatal("zero-value preflight result should be treated as timeout")
	}

	if isTimedOutPreflightResult(&common.StreamPreflightResult{BufferedEvents: []string{"event"}}) {
		t.Fatal("preflight result with buffered events should not be treated as timeout")
	}
}
