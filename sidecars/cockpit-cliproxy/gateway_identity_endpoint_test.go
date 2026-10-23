package main

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func gatewayEndpointManifest(t *testing.T) *manifest {
	t.Helper()
	second := json.RawMessage(strings.ReplaceAll(strings.ReplaceAll(string(gatewayIdentityFixture()), strings.Repeat("a", 64), strings.Repeat("b", 64)), "11111111-1111-4111-8111-111111111111", "22222222-2222-4222-8222-222222222222"))
	m := manifest{ModelIDs: []string{"gpt-5.4"}, Accounts: []accountSpec{
		{ID: "endpoint-a", AuthID: "a.json", AuthKind: "api_key", UpstreamAPIKey: "shared-key", UpstreamBaseURL: "https://vendor-a.invalid/v1", GatewayIdentity: gatewayIdentityFixture()},
		{ID: "endpoint-b", AuthID: "b.json", AuthKind: "api_key", UpstreamAPIKey: "shared-key", UpstreamBaseURL: "https://vendor-b.invalid/v1", GatewayIdentity: second},
	}}
	data, err := json.Marshal(m)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "manifest.json")
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
	loaded, err := loadManifest(path)
	if err != nil {
		t.Fatal(err)
	}
	return loaded
}

func TestGatewayIdentitySameKeyEndpointsRegisterAndKeepCallerScope(t *testing.T) {
	m := gatewayEndpointManifest(t)
	cfg := &config.Config{AuthDir: t.TempDir(), CodexKey: []config.CodexKey{
		{APIKey: "shared-key", BaseURL: "HTTPS://VENDOR-A.INVALID:443/v1/"},
		{APIKey: "shared-key", BaseURL: "https://vendor-b.invalid/v1"},
	}}
	manager := coreauth.NewManager(nil, nil, nil)
	service, err := cliproxy.NewBuilder().WithConfig(cfg).WithConfigPath(filepath.Join(cfg.AuthDir, "fixture.yaml")).WithCoreAuthManager(manager).WithHooks(cliproxy.Hooks{OnAuthPrepared: func(a *coreauth.Auth) error { return bindGatewayIdentityForManifestAuth(m, a) }}).Build()
	if err != nil {
		t.Fatal(err)
	}
	if err := initializeSidecarRuntimeAuths(context.Background(), service, cfg, m, manager); err != nil {
		t.Fatal("same-key endpoints failed startup registration", err)
	}
	auths := manager.List()
	if len(auths) != 2 {
		t.Fatal("wrong registered count", len(auths))
	}
	for _, a := range auths {
		expected := "endpoint-a"
		if strings.Contains(a.Attributes["base_url"], "vendor-b") {
			expected = "endpoint-b"
		}
		if a.Attributes["account_id"] != expected {
			t.Fatal("wrong bound account", a.Attributes["account_id"], expected)
		}
		if indexed := m.accountByAuthID[strings.ToLower(a.ID)]; indexed == nil || indexed.ID != expected {
			t.Fatal("runtime auth index points to key alias")
		}
		// A stale key-only index must not override the explicitly bound local account.
		other := "endpoint-b"
		if expected == other {
			other = "endpoint-a"
		}
		m.accountByAuthID[strings.ToLower(a.ID)] = m.accountByID[other]
		linkManifestAccountForAuth(m, a)
		if got := findManifestAccountForAuth(m, a); got == nil || got.ID != expected {
			t.Fatal("bound ID lost to stale index")
		}
		if m.accountByAuthID[strings.ToLower(a.ID)].ID != expected {
			t.Fatal("stale runtime index was not repaired")
		}
		raw := []byte(`{"input":[{"type":"reasoning","encrypted_content":"cipher-original","installation_id":"business"}],"metadata":{"installation_id":"business"}}`)
		body, err := helps.ApplyManagedIdentity(a, nil, raw)
		if err != nil || !bytes.Equal(body, raw) {
			t.Fatal("profile changed opaque history", err)
		}
	}
	selector := &cockpitSelector{manifest: m}
	for _, id := range []string{"endpoint-a", "endpoint-b"} {
		caller := &apiKeySpec{ID: "caller-" + id, AccountIDs: []string{id}}
		ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, caller)
		selected, err := selector.Pick(ctx, "codex", "gpt-5.4", coreexecutor.Options{}, auths)
		if err != nil || selected == nil {
			t.Fatal("caller selection failed", err)
		}
		if selected.Attributes["account_id"] != id {
			t.Fatal("caller escaped account scope", id, selected.Attributes["account_id"])
		}
	}
}

func TestGatewayIdentityEndpointPrecedenceAndCredentialConflicts(t *testing.T) {
	m := gatewayEndpointManifest(t)
	explicit := &coreauth.Auth{ID: "b.json", Provider: "codex", Attributes: map[string]string{"account_id": "endpoint-a", "api_key": "shared-key", "base_url": "HTTPS://VENDOR-A.INVALID:443/v1/"}}
	if err := bindGatewayIdentityForManifestAuth(m, explicit); err != nil {
		t.Fatal("explicit ID lost to file/key alias", err)
	}
	if explicit.Attributes[helps.IdentityScopeAttribute] != strings.Repeat("a", 64) {
		t.Fatal("wrong identity scope")
	}
	for _, change := range []struct{ field, value string }{{"api_key", "replaced-key"}, {"base_url", "https://vendor-b.invalid/v1"}, {"base_url", "https://vendor-a.invalid/V1"}, {"base_url", ""}, {"account_id", "unknown"}} {
		changed := explicit.Clone()
		changed.Attributes[change.field] = change.value
		if err := bindGatewayIdentityForManifestAuth(m, changed); err == nil {
			t.Fatal("old scope accepted changed credential/endpoint", change.field, change.value)
		}
	}
	byFile := &coreauth.Auth{ID: "a.json", Provider: "codex", Attributes: map[string]string{"api_key": "shared-key", "base_url": "https://vendor-a.invalid/v1"}}
	if err := bindGatewayIdentityForManifestAuth(m, byFile); err != nil || byFile.Attributes["account_id"] != "endpoint-a" {
		t.Fatal("exact file lost to key alias", err)
	}
	byFile.Attributes["base_url"] = "https://vendor-b.invalid/v1"
	delete(byFile.Attributes, "account_id")
	if err := bindGatewayIdentityForManifestAuth(m, byFile); err == nil {
		t.Fatal("exact file was rebound through key alias")
	}
	missingBase := &coreauth.Auth{ID: "unbound", Provider: "codex", Attributes: map[string]string{"api_key": "shared-key"}}
	m.Accounts[1].Provider = "xai"
	if err := bindGatewayIdentityForManifestAuth(m, missingBase); err == nil {
		t.Fatal("missing base or provider name resolved a real ambiguity")
	}

	// Missing endpoint projection retains compatibility; it must not invent one.
	oldProfile := gatewayEndpointManifest(t)
	oldProfile.Accounts[0].UpstreamBaseURL = ""
	compatible := explicit.Clone()
	compatible.Attributes["base_url"] = ""
	if err := bindGatewayIdentityForManifestAuth(oldProfile, compatible); err != nil {
		t.Fatal("old optional endpoint projection changed behavior", err)
	}
	for i := range m.Accounts {
		m.Accounts[i].GatewayIdentity = nil
	}
	legacy := &coreauth.Auth{ID: "legacy", Provider: "codex", Attributes: map[string]string{"account_id": "endpoint-a", "api_key": "shared-key", "base_url": "https://vendor-a.invalid/v1"}}
	if err := bindGatewayIdentityForManifestAuth(m, legacy); err != nil {
		t.Fatal("legacy binding changed", err)
	}
	if got := findManifestAccountForAuth(m, legacy); got != m.accountByAPIKey["shared-key"] {
		t.Fatal("legacy find precedence changed")
	}
}

func TestGatewayIdentityDirectEndpointAndImageException(t *testing.T) {
	m := gatewayEndpointManifest(t)
	s := &relayServer{manifest: m}
	for _, tc := range []struct {
		id, key, base string
		want          string
	}{
		{"endpoint-a", "shared-key", "HTTPS://VENDOR-A.INVALID:443/v1/", "endpoint-a"},
		{"", "shared-key", "https://vendor-b.invalid/v1", "endpoint-b"},
		{"endpoint-a", "shared-key", "https://vendor-b.invalid/v1", ""},
		{"endpoint-a", "wrong-key", "https://vendor-a.invalid/v1", ""},
		{"missing", "shared-key", "https://vendor-a.invalid/v1", ""},
		{"", "shared-key", "", ""},
	} {
		auth, err := s.directGatewayIdentity(tc.id, tc.key, tc.base, true)
		if tc.want == "" {
			if err == nil {
				t.Fatal("invalid direct binding accepted", tc)
			}
			continue
		}
		if err != nil || auth == nil || auth.Attributes["account_id"] != tc.want {
			t.Fatal("valid direct binding failed", tc, err)
		}
	}
	m.Accounts[0].ImageGenerationUpstream = &imageAPIUpstreamSpec{APIKey: "image-key", BaseURL: "https://image-vendor.invalid/v1"}
	if _, err := s.directGatewayIdentity("endpoint-a", "image-key", "HTTPS://IMAGE-VENDOR.INVALID:443/v1/", false); err != nil {
		t.Fatal("declared image credential rejected", err)
	}
	if _, err := s.directGatewayIdentity("endpoint-a", "image-key", "https://unrelated.invalid/v1", false); err == nil {
		t.Fatal("image key bypassed endpoint ownership")
	}
}

func TestGatewayIdentityCanonicalBaseURL(t *testing.T) {
	for _, tc := range []struct {
		first, second string
		equal         bool
	}{
		{"HTTPS://EXAMPLE.COM:443/v1/", "https://example.com/v1", true},
		{"HTTP://EXAMPLE.COM:80/v1///", "http://example.com/v1", true},
		{"https://api.openai.com:443/v1/", "https://api.openai.com/v1", true},
		{"https://example.com/V1", "https://example.com/v1", false},
		{"https://example.com:444/v1", "https://example.com/v1", false},
		{"", "https://api.openai.com/v1", false},
		{"socks5://example.com/v1", "socks5://example.com/v1", false},
	} {
		if got := gatewayBaseURLsMatch(tc.first, tc.second); got != tc.equal {
			t.Fatal(tc, got)
		}
	}
}
