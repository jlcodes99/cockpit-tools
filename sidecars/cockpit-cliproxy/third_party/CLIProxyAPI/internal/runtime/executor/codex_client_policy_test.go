package executor

import (
	"net/http"
	"testing"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestEnforceCodexClientPolicy(t *testing.T) {
	tests := []struct {
		name          string
		metadata      map[string]any
		userAgent     string
		originator    string
		wantForbidden bool
	}{
		{
			name:          "policy disabled",
			metadata:      map[string]any{},
			userAgent:     "curl/8.0",
			wantForbidden: false,
		},
		{
			name:          "official ua",
			metadata:      map[string]any{"codex_cli_only": true},
			userAgent:     "codex-tui/0.146.0 (macOS; arm64)",
			wantForbidden: false,
		},
		{
			name:          "official originator",
			metadata:      map[string]any{"codex_cli_only": true},
			userAgent:     "curl/8.0",
			originator:    "codex_cli_rs",
			wantForbidden: false,
		},
		{
			name:          "account app server override",
			metadata:      map[string]any{"codex_cli_only": true, "codex_cli_only_allow_app_server": true},
			userAgent:     "claude-code/1.0",
			wantForbidden: false,
		},
		{
			name:          "global app server override",
			metadata:      map[string]any{"codex_cli_only": true, "codex_cli_only_allow_app_server_clients": true},
			userAgent:     "claude-code/1.0",
			wantForbidden: false,
		},
		{
			name:          "unknown client rejected",
			metadata:      map[string]any{"codex_cli_only": true},
			userAgent:     "curl/8.0",
			wantForbidden: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			headers := make(http.Header)
			headers.Set("User-Agent", tt.userAgent)
			headers.Set("Originator", tt.originator)
			err := enforceCodexClientPolicy(&cliproxyauth.Auth{Metadata: tt.metadata}, headers, nil)
			if (err != nil) != tt.wantForbidden {
				t.Fatalf("forbidden = %v, want %v; err=%v", err != nil, tt.wantForbidden, err)
			}
		})
	}
}
