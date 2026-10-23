package main

import (
	"net"
	"net/http"
	"net/url"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func validateGatewayIdentityManifest(m *manifest) error {
	seenAccounts := map[string]bool{}
	seenScopes := map[string]struct{}{}
	seenInstallations := map[string]struct{}{}
	for i := range m.Accounts {
		account := &m.Accounts[i]
		profile, err := helps.ParseManagedIdentity(account.GatewayIdentity)
		if err != nil {
			return err
		}
		accountID := strings.TrimSpace(account.ID)
		if managed, seen := seenAccounts[accountID]; seen && (managed || profile != nil) {
			return helps.IdentityError{}
		}
		seenAccounts[accountID] = profile != nil
		if profile == nil {
			continue
		}
		if accountID == "" {
			return helps.IdentityError{}
		}
		if _, seen := seenScopes[profile.Scope]; seen {
			return helps.IdentityError{}
		}
		if _, seen := seenInstallations[profile.InstallationID]; seen {
			return helps.IdentityError{}
		}
		seenScopes[profile.Scope] = struct{}{}
		seenInstallations[profile.InstallationID] = struct{}{}
	}
	return nil
}
func bindGatewayIdentityAuth(auth *coreauth.Auth, account *accountSpec) error {
	if account == nil {
		return nil
	}
	profile, err := helps.ParseManagedIdentity(account.GatewayIdentity)
	if err != nil {
		return err
	}
	if profile != nil {
		if err := helps.ValidateManagedCodexPrincipal(auth, account.ChatGPTUserID, account.ChatGPTAccountID); err != nil {
			return err
		}
	}
	if err := helps.BindManagedIdentity(auth, account.ID, profile); err != nil {
		return err
	}
	if profile != nil {
		auth.Attributes[helps.IdentityUserAttribute] = account.ChatGPTUserID
		auth.Attributes[helps.IdentityAccountAttribute] = account.ChatGPTAccountID
	}
	return nil
}

func manifestUsesGatewayIdentity(m *manifest) bool {
	if m != nil {
		for i := range m.Accounts {
			if len(m.Accounts[i].GatewayIdentity) > 0 {
				return true
			}
		}
	}
	return false
}

func gatewayManifestLocalAccount(m *manifest, id string) *accountSpec {
	if m == nil {
		return nil
	}
	if account := m.accountByID[id]; account != nil {
		return account
	}
	for i := range m.Accounts {
		if m.Accounts[i].ID == id {
			return &m.Accounts[i]
		}
	}
	return nil
}

// Match the host's URL canonicalization without changing case-sensitive paths.
func canonicalGatewayBaseURL(raw string) (string, bool) {
	parsed, err := url.Parse(strings.TrimSpace(raw))
	if err != nil || parsed.Host == "" {
		return "", false
	}
	parsed.Scheme = strings.ToLower(parsed.Scheme)
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return "", false
	}
	host := strings.ToLower(parsed.Hostname())
	if host == "" {
		return "", false
	}
	port := parsed.Port()
	if port != "" {
		number, err := strconv.ParseUint(port, 10, 16)
		if err != nil {
			return "", false
		}
		port = strconv.FormatUint(number, 10)
		if (parsed.Scheme == "http" && port == "80") || (parsed.Scheme == "https" && port == "443") {
			port = ""
		}
	}
	if port != "" {
		parsed.Host = net.JoinHostPort(host, port)
	} else if strings.Contains(host, ":") {
		parsed.Host = "[" + host + "]"
	} else {
		parsed.Host = host
	}
	return strings.TrimRight(parsed.String(), "/"), true
}

func gatewayBaseURLsMatch(actual, declared string) bool {
	first, firstOK := canonicalGatewayBaseURL(actual)
	second, secondOK := canonicalGatewayBaseURL(declared)
	return firstOK && secondOK && first == second
}

func validateGatewayIdentityUpstream(account *accountSpec, key, base string, allowImage bool) error {
	if account == nil || len(account.GatewayIdentity) == 0 {
		return nil
	}
	key = strings.TrimSpace(key)
	keyMatches := account.UpstreamAPIKey == "" || strings.TrimSpace(account.UpstreamAPIKey) == key
	baseMatches := strings.TrimSpace(account.UpstreamBaseURL) == "" || gatewayBaseURLsMatch(base, account.UpstreamBaseURL)
	if keyMatches && baseMatches {
		return nil
	}
	if allowImage && account.ImageGenerationUpstream != nil {
		image := account.ImageGenerationUpstream
		if key != "" && key == strings.TrimSpace(image.APIKey) && gatewayBaseURLsMatch(base, image.BaseURL) {
			return nil
		}
	}
	return helps.IdentityError{}
}

// Resolve an unbound config auth by the complete available credential/endpoint
// pair. Missing endpoint evidence cannot distinguish same-key accounts.
func gatewayManifestKeyAccount(m *manifest, key, base string) (*accountSpec, error) {
	key = strings.TrimSpace(key)
	if key == "" {
		return nil, nil
	}
	var matched *accountSpec
	for i := range m.Accounts {
		account := &m.Accounts[i]
		if strings.TrimSpace(account.UpstreamAPIKey) != key {
			continue
		}
		if strings.TrimSpace(base) != "" && strings.TrimSpace(account.UpstreamBaseURL) != "" && !gatewayBaseURLsMatch(base, account.UpstreamBaseURL) {
			continue
		}
		if matched != nil && matched.ID != account.ID {
			return nil, helps.IdentityError{}
		}
		matched = account
	}
	return matched, nil
}

// Trusted local IDs and exact auth-file IDs outrank API-key aliases. Provider
// names and email/workspace claims cannot disambiguate separately imported accounts.
func gatewayManifestAccountForAuth(m *manifest, auth *coreauth.Auth) (*accountSpec, error) {
	if m == nil || auth == nil {
		return nil, nil
	}
	finish := func(account *accountSpec) (*accountSpec, error) {
		if err := validateGatewayIdentityUpstream(account, auth.Attributes["api_key"], auth.Attributes["base_url"], false); err != nil {
			return nil, err
		}
		return account, nil
	}
	if id := strings.TrimSpace(auth.Attributes["account_id"]); id != "" {
		account := gatewayManifestLocalAccount(m, id)
		if account == nil {
			return nil, helps.IdentityError{}
		}
		return finish(account)
	}
	candidates := []string{auth.ID, auth.FileName, auth.Attributes["path"]}
	for _, basenameOnly := range []bool{false, true} {
		var matched *accountSpec
		for i := range m.Accounts {
			account := &m.Accounts[i]
			file := strings.TrimSpace(account.AuthID)
			if file == "" {
				continue
			}
			for _, candidate := range candidates {
				candidate = strings.TrimSpace(candidate)
				if candidate == "" {
					continue
				}
				expected := file
				if basenameOnly {
					expected = filepath.Base(expected)
					candidate = filepath.Base(candidate)
				}
				if !strings.EqualFold(expected, candidate) {
					continue
				}
				if matched != nil && matched.ID != account.ID {
					return nil, helps.IdentityError{}
				}
				matched = account
				break
			}
		}
		if matched != nil {
			return finish(matched)
		}
	}
	matched, err := gatewayManifestKeyAccount(m, auth.Attributes["api_key"], auth.Attributes["base_url"])
	if err != nil {
		return nil, err
	}
	return finish(matched)
}

func bindGatewayIdentityForManifestAuth(m *manifest, auth *coreauth.Auth) error {
	if m == nil || auth == nil || !manifestUsesGatewayIdentity(m) {
		return nil
	}
	account, err := gatewayManifestAccountForAuth(m, auth)
	if err != nil {
		return err
	}
	if account != nil {
		return bindGatewayIdentityAuth(auth, account)
	}
	if profile, err := helps.ManagedIdentityForAuth(auth); err != nil || profile != nil {
		return helps.IdentityError{}
	}
	for i := range m.Accounts {
		account := &m.Accounts[i]
		provider := strings.TrimSpace(account.Provider)
		if provider == "" {
			provider = "codex"
		}
		if len(account.GatewayIdentity) > 0 && strings.EqualFold(provider, auth.Provider) {
			return helps.IdentityError{}
		}
	}
	return nil
}

func managedGatewayRoundTripper(auth *coreauth.Auth) (http.RoundTripper, bool) {
	profile, err := helps.ManagedIdentityForAuth(auth)
	if profile == nil && err == nil {
		return nil, false
	}
	return helps.ManagedStandardTransport(auth, auth.ProxyURL, "sdk"), true
}

// Direct provider requests use an explicit selected account, never a caller's
// device header. Non-Codex schemas keep all their business JSON unchanged.
func (s *relayServer) directGatewayIdentity(accountID, credential, endpoint string, codex bool) (*coreauth.Auth, error) {
	if s == nil || s.manifest == nil {
		return nil, nil
	}
	accountID = strings.TrimSpace(accountID)
	account := gatewayManifestLocalAccount(s.manifest, accountID)
	managed := manifestUsesGatewayIdentity(s.manifest)
	if account == nil && accountID != "" && managed {
		return nil, helps.IdentityError{}
	}
	if account == nil {
		if managed {
			var err error
			account, err = gatewayManifestKeyAccount(s.manifest, credential, endpoint)
			if err != nil {
				return nil, err
			}
		} else {
			for i := range s.manifest.Accounts {
				candidate := &s.manifest.Accounts[i]
				if credential != "" && candidate.UpstreamAPIKey == credential {
					account = candidate
				}
			}
		}
	}
	if account == nil {
		if managed {
			return nil, helps.IdentityError{}
		}
		return nil, nil
	}
	if err := validateGatewayIdentityUpstream(account, credential, endpoint, true); err != nil {
		return nil, err
	}
	provider := "openai-compatible"
	if codex {
		provider = "codex"
	}
	auth := &coreauth.Auth{ID: account.ID, Provider: provider, Attributes: map[string]string{"account_id": account.ID, "api_key": credential, "base_url": endpoint}}
	if err := bindGatewayIdentityAuth(auth, account); err != nil {
		return nil, err
	}
	if len(account.GatewayIdentity) > 0 {
		auth.ProxyURL = strings.TrimSpace(account.UpstreamProxyURL)
	}
	return auth, nil
}
func applyDirectGatewayIdentity(req *http.Request, auth *coreauth.Auth) (*http.Client, error) {
	if err := helps.ApplyManagedIdentityRequest(req, auth); err != nil {
		return nil, err
	}
	profile, err := helps.ManagedIdentityForAuth(auth)
	if err != nil {
		return nil, err
	}
	if profile == nil {
		return nil, nil
	}
	return &http.Client{Transport: helps.ManagedStandardTransport(auth, auth.ProxyURL, "direct:"+req.URL.Scheme+"://"+req.URL.Host), CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, nil
}
