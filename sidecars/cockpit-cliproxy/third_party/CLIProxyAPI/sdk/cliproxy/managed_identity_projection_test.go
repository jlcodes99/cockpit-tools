package cliproxy

import (
	"context"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/watcher"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"strings"
	"testing"
)

func TestManagedIdentityProjectionSurvivesRuntimeAndWatcherUpdates(t *testing.T) {
	manager := coreauth.NewManager(nil, nil, nil)
	profile := &helps.ManagedIdentity{Version: 1, Scope: strings.Repeat("a", 64), InstallationID: "11111111-1111-4111-8111-111111111111", Revision: 1}
	service := &Service{cfg: &config.Config{}, coreManager: manager, hooks: Hooks{OnAuthPrepared: func(a *coreauth.Auth) error { return helps.BindManagedIdentity(a, "account-a", profile) }}}
	auth := &coreauth.Auth{ID: "fixture", Provider: "codex", Metadata: map[string]any{"access_token": "first"}}
	first, err := service.UpsertRuntimeAuth(context.Background(), auth)
	if err != nil {
		t.Fatal(err)
	}
	firstKey, _, _ := helps.ManagedTransportKey(first, "", "standard")
	auth.Metadata["access_token"] = "refreshed"
	service.handleAuthUpdate(context.Background(), watcher.AuthUpdate{Action: watcher.AuthUpdateActionModify, Auth: auth})
	current, ok := manager.GetByID(auth.ID)
	if !ok {
		t.Fatal("missing auth")
	}
	if current.Attributes[helps.InstallationIDAttribute] != profile.InstallationID || current.Attributes["account_id"] != "account-a" {
		t.Fatal("projection lost")
	}
	nextKey, _, _ := helps.ManagedTransportKey(current, "", "standard")
	if firstKey == nextKey {
		t.Fatal("new credential reused pool key")
	}
	service.hooks.OnAuthPrepared = func(*coreauth.Auth) error { return helps.IdentityError{} }
	auth.Metadata["access_token"] = "rejected"
	service.handleAuthUpdate(context.Background(), watcher.AuthUpdate{Action: watcher.AuthUpdateActionModify, Auth: auth})
	current, _ = manager.GetByID(auth.ID)
	if current.Metadata["access_token"] != "refreshed" {
		t.Fatal("rejected update installed")
	}
}
