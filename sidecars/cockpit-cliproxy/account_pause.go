package main

import (
	"context"
	"errors"
	"net/http"
	"strings"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func normalizePausedAccountIDs(ids []string) []string {
	seen := make(map[string]struct{}, len(ids))
	result := make([]string, 0, len(ids))
	for _, id := range ids {
		id = strings.TrimSpace(id)
		if _, exists := seen[id]; id == "" || exists {
			continue
		}
		seen[id] = struct{}{}
		result = append(result, id)
	}
	return result
}

// Pausing is a service scheduling policy, not a credential mutation. Keep both
// manifest accounts and API-key scopes intact, especially an all-paused scope:
// an empty AccountIDs slice has the separate meaning of unrestricted access.
func serviceAccountPaused(ctx context.Context, m *manifest, accountID string) bool {
	if m == nil || len(m.PausedAccountIDs) == 0 {
		return false
	}
	if ctx != nil {
		if key, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec); key != nil && key.Internal {
			return false
		}
	}
	accountID = strings.TrimSpace(accountID)
	if accountID == "" {
		return false
	}
	for _, paused := range m.PausedAccountIDs {
		if strings.TrimSpace(paused) == accountID {
			return true
		}
	}
	return false
}

func serviceAuthPaused(ctx context.Context, m *manifest, auth *coreauth.Auth) bool {
	account := accountForAuthInManifest(m, auth)
	return account != nil && serviceAccountPaused(ctx, m, account.ID)
}

func unpausedServiceAuths(ctx context.Context, m *manifest, auths []*coreauth.Auth) []*coreauth.Auth {
	if m == nil || len(m.PausedAccountIDs) == 0 {
		return auths
	}
	active := make([]*coreauth.Auth, 0, len(auths))
	for _, auth := range auths {
		if !serviceAuthPaused(ctx, m, auth) {
			active = append(active, auth)
		}
	}
	return active
}

func serviceAccountPausedError() error {
	return &coreauth.Error{
		Code: "account_paused", Message: "selected accounts are paused in this service",
		HTTPStatus: http.StatusServiceUnavailable,
	}
}

// A deliberate pause of every candidate in an explicit key scope needs user
// action. The existing SDK terminal marker exposes the error to WS clients
// rather than silently reconnecting forever. Unrestricted scopes stay unchanged.
func terminalScopedPauseError(ctx context.Context, err error) error {
	if ctx == nil || len(apiKeyScopeAccountIDs(ctx)) == 0 {
		return err
	}
	var diagnostic *coreauth.Error
	if !errors.As(err, &diagnostic) || diagnostic.Code != "account_paused" {
		return err
	}
	copy := *diagnostic
	copy.Retryable = false
	return coreauth.NewTerminalAuthError(&copy, err)
}

// Also enforce the policy immediately before a generation send, including
// pinned WebSocket continuations. Preserve the existing quota admission guard.
func (p *requestPolicy) beforeUpstreamSend(ctx context.Context, authID string) error {
	if serviceAuthPaused(ctx, p.manifest, &coreauth.Auth{ID: authID}) {
		return serviceAccountPausedError()
	}
	if p.tracker != nil {
		return p.tracker.quotaProtectionBeforeSend(ctx, authID)
	}
	return nil
}
