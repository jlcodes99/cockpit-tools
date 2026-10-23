package helps

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
)

func managedFixture(t *testing.T, account string) *coreauth.Auth {
	t.Helper()
	a := &coreauth.Auth{ID: account, Provider: "codex", Attributes: map[string]string{"api_key": "fixture"}}
	if err := BindManagedIdentity(a, account, &ManagedIdentity{1, strings.Repeat("a", 64), "11111111-1111-4111-8111-111111111111", 1}); err != nil {
		t.Fatal(err)
	}
	return a
}
func TestManagedIdentityValidationAndLegacy(t *testing.T) {
	for _, raw := range []string{`null`, `{}`, `{"version":2,"scope":"a","revision":1}`, `{"version":1,"scope":"` + strings.Repeat("a", 64) + `","installationId":"11111111-1111-5111-8111-111111111111","revision":1}`} {
		if _, err := ParseManagedIdentity(json.RawMessage(raw)); err == nil {
			t.Fatalf("accepted %s", raw)
		}
	}
	if p, err := ParseManagedIdentity(nil); p != nil || err != nil {
		t.Fatal(p, err)
	}
	raw := []byte(`{"previous_response_id":"response-old","installation_id":"legacy","input":[{"encrypted_content":"cipher-old"}]}`)
	headers := http.Header{"x-codex-installation-id": {"old"}}
	got, err := ApplyManagedIdentity(&coreauth.Auth{ID: "legacy"}, headers, raw)
	if err != nil || !bytes.Equal(got, raw) || headers["x-codex-installation-id"][0] != "old" {
		t.Fatal("legacy request changed")
	}
}
func TestManagedIdentityExactPathsAndOpaqueHistory(t *testing.T) {
	a := managedFixture(t, "a")
	raw := []byte(`{"installation_id":"client","prompt_cache_key":"session-old","previous_response_id":"response-old","client_metadata":{"x-codex-installation-id":"client","session_id":"session-old","x-codex-turn-metadata":"{\"installation_id\":\"client\",\"turn_id\":\"turn-old\"}"},"metadata":{"installation_id":"client","business":{"installation_id":"do-not-touch"}},"input":[{"type":"reasoning","encrypted_content":"cipher-old","installation_id":"do-not-touch"}],"tools":[{"parameters":{"installation_id":"do-not-touch"}}]}`)
	headers := http.Header{"x-codex-installation-id": {"spoof"}, "X-Codex-Installation-Id": {"spoof2"}, "Session-Id": {"session-old"}, "User-Agent": {"client-version"}}
	got, err := ApplyManagedIdentity(a, headers, raw)
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"installation_id", "client_metadata.x-codex-installation-id"} {
		if gjson.GetBytes(got, path).String() != a.Attributes[InstallationIDAttribute] {
			t.Fatal(path)
		}
	}
	for _, path := range []string{"prompt_cache_key", "previous_response_id", "input", "tools", "metadata", "client_metadata.session_id"} {
		if gjson.GetBytes(got, path).Raw != gjson.GetBytes(raw, path).Raw {
			t.Fatal("opaque path changed", path)
		}
	}
	if headers.Get("User-Agent") != "client-version" || headers.Get("Session-Id") != "session-old" {
		t.Fatal(headers)
	}
	count := 0
	for k := range headers {
		if strings.EqualFold(k, "X-Codex-Installation-Id") {
			count++
		}
	}
	if count != 1 {
		t.Fatal(headers)
	}
	a.Provider = "xai"
	other, _ := ApplyManagedIdentity(a, http.Header{}, raw)
	if !bytes.Equal(other, raw) {
		t.Fatal("non-Codex JSON changed")
	}
}
func TestManagedTransportIsolationRotationAndActiveBody(t *testing.T) {
	a := managedFixture(t, "a")
	b := managedFixture(t, "b")
	first := ManagedStandardTransport(a, "", "test")
	if first != ManagedStandardTransport(a, "", "test") || first == ManagedStandardTransport(b, "", "test") {
		t.Fatal("account pool isolation")
	}
	a.Metadata = map[string]any{"agent_assertion": "one"}
	if first != ManagedStandardTransport(a, "", "test") {
		t.Fatal("transient assertion rotated pool")
	}
	a.Attributes[IdentityRevisionAttribute] = "2"
	if first == ManagedStandardTransport(a, "", "test") {
		t.Fatal("revision did not rotate")
	}
	second := ManagedStandardTransport(a, "", "test")
	a.Attributes["api_key"] = "changed"
	if second == ManagedStandardTransport(a, "", "test") {
		t.Fatal("credential did not rotate")
	}
	started := make(chan struct{})
	release := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("first"))
		w.(http.Flusher).Flush()
		close(started)
		<-release
		_, _ = w.Write([]byte("last"))
	}))
	defer server.Close()
	client := &http.Client{Transport: ManagedStandardTransport(a, "", "active")}
	response, err := client.Get(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	<-started
	a.Attributes[IdentityRevisionAttribute] = "3"
	_ = ManagedStandardTransport(a, "", "active")
	close(release)
	body, err := io.ReadAll(response.Body)
	_ = response.Body.Close()
	if err != nil || string(body) != "firstlast" {
		t.Fatal(string(body), err)
	}
	for i := 0; i < managedPoolLimit+8; i++ {
		a.Attributes["base_url"] = strings.Repeat("x", i)
		_ = ManagedStandardTransport(a, "", string(rune(i))+"bounded")
	}
	managedPools.Lock()
	n := managedPools.order.Len()
	managedPools.Unlock()
	if n > managedPoolLimit {
		t.Fatal(n)
	}
}

func TestManagedIdentityRepeatedKnownKeysAndMultipart(t *testing.T) {
	a := managedFixture(t, "duplicate")
	raw := []byte(`{"installation_id":"spoof1","installation_id":"spoof2","client_metadata":{"installation_id":"spoof3"},"client_metadata":{"installation_id":"spoof4"},"input":{"installation_id":"business"}}`)
	got, err := ApplyManagedIdentity(a, nil, raw)
	if err != nil || strings.Contains(string(got), "spoof") || !strings.Contains(string(got), `"installation_id":"business"`) {
		t.Fatal(string(got), err)
	}
	req, _ := http.NewRequest("POST", "http://example.invalid", strings.NewReader("multipart unchanged"))
	req.Header.Set("Content-Type", "multipart/form-data; boundary=fixture")
	if err := ApplyManagedIdentityRequest(req, a); err != nil {
		t.Fatal(err)
	}
	payload, _ := io.ReadAll(req.Body)
	if string(payload) != "multipart unchanged" {
		t.Fatal(string(payload))
	}
}

func TestManagedIdentityRefreshedJWTPrincipalMustMatchProjection(t *testing.T) {
	a := managedFixture(t, "claims")
	delete(a.Attributes, "api_key")
	a.Attributes[IdentityUserAttribute] = "user-a"
	a.Attributes[IdentityAccountAttribute] = "team-a"
	token := func(user, team string) string {
		return "header." + base64.RawURLEncoding.EncodeToString([]byte(`{"https://api.openai.com/auth":{"chatgpt_user_id":"`+user+`","chatgpt_account_id":"`+team+`"}}`)) + ".signature"
	}
	a.Metadata = map[string]any{"access_token": token("user-a", "team-a")}
	if _, err := ManagedIdentityForAuth(a); err != nil {
		t.Fatal(err)
	}
	for _, value := range []string{token("user-b", "team-a"), token("user-a", "team-b")} {
		a.Metadata["access_token"] = value
		if _, err := ManagedIdentityForAuth(a); err == nil {
			t.Fatal("changed JWT principal retained scope")
		}
	}
}

func TestManagedTransportConfigurationAndHandshakeChanges(t *testing.T) {
	a := managedFixture(t, "transport-config")
	first := ManagedStandardTransport(a, "", "config-test")
	if first == ManagedStandardTransport(a, "direct", "config-test") {
		t.Fatal("proxy configuration reused pool")
	}
	if first == ManagedStandardTransport(a, "", "other-protocol") {
		t.Fatal("protocol reused pool")
	}
	a.Attributes["base_url"] = "http://other.invalid"
	if first == ManagedStandardTransport(a, "", "config-test") {
		t.Fatal("endpoint reused pool")
	}
	headers := http.Header{"Authorization": {"AgentAssertion one"}, "User-Agent": {"version1"}}
	key, err := ManagedWebsocketKey(a, "", "wss://fixture.invalid/responses", headers)
	if err != nil {
		t.Fatal(err)
	}
	headers.Set("Authorization", "AgentAssertion two")
	headers.Set("Session-Id", "another-session")
	headers.Set("X-Codex-Turn-Metadata", `{"turn_id":"another-turn"}`)
	same, _ := ManagedWebsocketKey(a, "", "wss://fixture.invalid/responses", headers)
	if same != key {
		t.Fatal("transient assertion or request identifiers changed handshake generation")
	}
	headers.Set("User-Agent", "version2")
	changed, _ := ManagedWebsocketKey(a, "", "wss://fixture.invalid/responses", headers)
	if changed == key {
		t.Fatal("stable handshake change ignored")
	}
	a.Attributes[IdentityRevisionAttribute] = "0"
	if _, err := ManagedStandardTransport(a, "", "config-test").RoundTrip(&http.Request{}); err == nil {
		t.Fatal("invalid profile reached transport")
	}
}

func TestManagedCodexCookieBoundaryAndAgentTaskGeneration(t *testing.T) {
	headers := http.Header{"cookie": {"caller-session"}, "Cookie": {"custom-session"}, "COOKIE2": {"legacy-session"}, "Session-Id": {"conversation"}}
	legacy := &coreauth.Auth{Provider: "codex"}
	if _, err := ApplyManagedIdentity(legacy, headers, nil); err != nil || headers["cookie"][0] != "caller-session" {
		t.Fatal("legacy cookie behavior changed")
	}
	managed := managedFixture(t, "cookies")
	if _, err := ApplyManagedIdentity(managed, headers, nil); err != nil {
		t.Fatal(err)
	}
	for name := range headers {
		if strings.EqualFold(name, "cookie") || strings.EqualFold(name, "cookie2") {
			t.Fatal("cookie survived", name)
		}
	}
	if headers.Get("Session-Id") != "conversation" {
		t.Fatal("session changed")
	}
	before, _ := ManagedIdentityForAuth(managed)
	key, _, _ := ManagedTransportKey(managed, "", "task")
	for _, field := range []string{"task_id", "taskId"} {
		changed := managed.Clone()
		changed.Attributes[field] = "task-next"
		next, _, _ := ManagedTransportKey(changed, "", "task")
		if next == key {
			t.Fatal("task attribute ignored", field)
		}
		changed = managed.Clone()
		changed.Metadata = map[string]any{field: "task-next"}
		next, _, _ = ManagedTransportKey(changed, "", "task")
		if next == key {
			t.Fatal("task metadata ignored", field)
		}
		after, err := ManagedIdentityForAuth(changed)
		if err != nil || *before != *after {
			t.Fatal("task changed installation profile")
		}
	}
}

func TestManagedIdentityChecksOnlySelectedBearer(t *testing.T) {
	token := func(user, team string) string {
		return "header." + base64.RawURLEncoding.EncodeToString([]byte(`{"https://api.openai.com/auth":{"chatgpt_user_id":"`+user+`","chatgpt_account_id":"`+team+`"}}`)) + ".signature"
	}
	valid := token("user-a", "team-a")
	conflicting := token("user-old", "team-old")
	for _, tc := range []struct {
		name, apiKey, access string
		rejected             bool
	}{
		{"new_access_ignores_old_id_and_alias_tokens", "", valid, false},
		{"opaque_access_ignores_old_id_token", "", "opaque-access", false},
		{"api_key_wins_over_unused_access_token", valid, conflicting, false},
		{"opaque_api_key_wins_over_unused_access_token", "opaque-api-key", conflicting, false},
		{"actual_access_conflict", "", conflicting, true},
		{"actual_api_key_conflict", conflicting, valid, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			a := managedFixture(t, "bearer-selection")
			a.Attributes["api_key"] = tc.apiKey
			a.Attributes[IdentityUserAttribute] = "user-a"
			a.Attributes[IdentityAccountAttribute] = "team-a"
			a.Metadata = map[string]any{"access_token": tc.access, "id_token": conflicting, "personal_access_token": conflicting, "at_token": conflicting, "account_id": "team-a", "chatgpt_user_id": "user-a"}
			_, err := ManagedIdentityForAuth(a)
			if (err != nil) != tc.rejected {
				t.Fatalf("rejected=%t want=%t", err != nil, tc.rejected)
			}
		})
	}
	for _, field := range []string{"account_id", "chatgpt_user_id"} {
		a := managedFixture(t, "explicit-claims")
		delete(a.Attributes, "api_key")
		a.Attributes[IdentityUserAttribute] = "user-a"
		a.Attributes[IdentityAccountAttribute] = "team-a"
		a.Metadata = map[string]any{"auth_mode": "agentIdentity", field: "conflicting-principal"}
		if _, err := ManagedIdentityForAuth(a); err == nil {
			t.Fatal("explicit Agent Identity conflict accepted", field)
		}
		a.Metadata["access_token"] = "opaque-access"
		if _, err := ManagedIdentityForAuth(a); err == nil {
			t.Fatal("explicit metadata conflict accepted with opaque bearer", field)
		}
	}
	legacy := &coreauth.Auth{Provider: "codex", Attributes: map[string]string{"api_key": conflicting}, Metadata: map[string]any{"id_token": conflicting, "access_token": valid, "account_id": "old-team"}}
	if profile, err := ManagedIdentityForAuth(legacy); profile != nil || err != nil {
		t.Fatal("legacy behavior changed", err)
	}
}
