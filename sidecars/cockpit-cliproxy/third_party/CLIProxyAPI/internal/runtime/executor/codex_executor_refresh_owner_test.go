package executor

import (
	"context"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestCodexExecutorRefreshDefersToCockpitTokenAuthority(t *testing.T) {
	executor := NewCodexExecutor(&config.Config{})
	auth := &cliproxyauth.Auth{
		Metadata: map[string]any{
			"refresh_owner": "cockpit_token_authority",
			"refresh_token": "must-not-be-used",
		},
	}

	refreshed, err := executor.Refresh(context.Background(), auth)
	if err != nil {
		t.Fatalf("Refresh() error = %v", err)
	}
	if refreshed != auth {
		t.Fatal("Refresh() must return the existing auth without rotating credentials")
	}
}
