package providers

import (
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
)

func TestResponsesProvider_BuildProviderRequestBody_DoesNotCreateSessionWithoutPreviousResponseID(t *testing.T) {
	manager := session.NewSessionManager(time.Hour, 100, 100000)
	provider := &ResponsesProvider{SessionManager: manager}
	upstream := &config.UpstreamConfig{ServiceType: "openai"}

	_, _, err := provider.buildProviderRequestBody(nil, "/v1/responses", []byte(`{"model":"gpt-5","input":"hello"}`), upstream)
	if err != nil {
		t.Fatalf("buildProviderRequestBody() err = %v", err)
	}

	stats := manager.GetStats()
	if got := stats["total_sessions"]; got != 0 {
		t.Fatalf("total_sessions = %v, want 0", got)
	}
}
