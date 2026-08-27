package auth

import (
	"context"
	"fmt"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

// CodebuddyAuthenticator registers the CodeBuddy provider with the auth manager.
//
// CodeBuddy OAuth is completed by the host application (the Tauri orchestrator),
// which writes the resulting credentials directly into the sidecar's auths/
// directory. The CLI login flow is therefore intentionally unsupported; this
// authenticator exists so the provider key resolves for refresh-lead lookups and
// for consistency with the other providers.
type CodebuddyAuthenticator struct{}

// NewCodebuddyAuthenticator constructs a Codebuddy authenticator.
func NewCodebuddyAuthenticator() *CodebuddyAuthenticator {
	return &CodebuddyAuthenticator{}
}

// Provider returns the provider identifier.
func (a *CodebuddyAuthenticator) Provider() string {
	return "codebuddy"
}

// RefreshLead returns how far ahead of expiry a refresh should be triggered.
func (a *CodebuddyAuthenticator) RefreshLead() *time.Duration {
	return new(24 * time.Hour)
}

// Login is not supported for CodeBuddy; credentials are injected by the host app.
func (a *CodebuddyAuthenticator) Login(ctx context.Context, cfg *config.Config, opts *LoginOptions) (*coreauth.Auth, error) {
	return nil, fmt.Errorf("codebuddy: credentials are injected externally by the host app; CLI login is not supported")
}
