package helps

import (
	"bytes"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"sort"
	"strconv"
	"strings"

	"github.com/google/uuid"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
)

const (
	IdentityScopeAttribute    = "cockpit_identity_scope"
	InstallationIDAttribute   = "cockpit_installation_id"
	IdentityRevisionAttribute = "cockpit_identity_revision"
	IdentityVersionAttribute  = "cockpit_identity_version"
	IdentityUserAttribute     = "cockpit_identity_user_id"
	IdentityAccountAttribute  = "cockpit_identity_account_id"
)

// ManagedIdentity is a read-only projection of the host's durable account profile.
// It never supplies session, conversation, turn, cache or encrypted-content IDs.
type ManagedIdentity struct {
	Version        uint64 `json:"version"`
	Scope          string `json:"scope"`
	InstallationID string `json:"installationId"`
	Revision       uint64 `json:"revision"`
}

type IdentityError struct{}

func (IdentityError) Error() string         { return "managed account identity is invalid or unavailable" }
func (IdentityError) StatusCode() int       { return http.StatusServiceUnavailable }
func (IdentityError) IsRequestScoped() bool { return true }

func (p *ManagedIdentity) Validate() error {
	if p == nil || p.Version != 1 || p.Revision == 0 || len(p.Scope) != 64 || p.Scope != strings.ToLower(p.Scope) {
		return IdentityError{}
	}
	if _, err := hex.DecodeString(p.Scope); err != nil {
		return IdentityError{}
	}
	id, err := uuid.Parse(p.InstallationID)
	if err != nil || id.Version() != 4 || id.Variant() != uuid.RFC4122 || id.String() != p.InstallationID {
		return IdentityError{}
	}
	return nil
}

func ParseManagedIdentity(raw json.RawMessage) (*ManagedIdentity, error) {
	if len(raw) == 0 {
		return nil, nil
	}
	var profile *ManagedIdentity
	if json.Unmarshal(raw, &profile) != nil || profile.Validate() != nil {
		return nil, IdentityError{}
	}
	return profile, nil
}

func BindManagedIdentity(auth *coreauth.Auth, account string, profile *ManagedIdentity) error {
	if profile == nil {
		return nil
	}
	if auth == nil || strings.TrimSpace(account) == "" || profile.Validate() != nil {
		return IdentityError{}
	}
	if auth.Attributes == nil {
		auth.Attributes = map[string]string{}
	}
	auth.Attributes["account_id"] = account
	auth.Attributes[IdentityVersionAttribute] = strconv.FormatUint(profile.Version, 10)
	auth.Attributes[IdentityScopeAttribute] = profile.Scope
	auth.Attributes[InstallationIDAttribute] = profile.InstallationID
	auth.Attributes[IdentityRevisionAttribute] = strconv.FormatUint(profile.Revision, 10)
	return nil
}

func ManagedIdentityForAuth(auth *coreauth.Auth) (*ManagedIdentity, error) {
	if auth == nil {
		return nil, nil
	}
	a := auth.Attributes
	present := false
	for _, key := range []string{IdentityVersionAttribute, IdentityScopeAttribute, InstallationIDAttribute, IdentityRevisionAttribute} {
		if _, exists := a[key]; exists {
			present = true
		}
	}
	if !present {
		return nil, nil
	}
	version, errVersion := strconv.ParseUint(a[IdentityVersionAttribute], 10, 64)
	revision, errRevision := strconv.ParseUint(a[IdentityRevisionAttribute], 10, 64)
	p := &ManagedIdentity{Version: version, Scope: a[IdentityScopeAttribute], InstallationID: a[InstallationIDAttribute], Revision: revision}
	if errVersion != nil || errRevision != nil || p.Validate() != nil || strings.TrimSpace(a["account_id"]) == "" {
		return nil, IdentityError{}
	}
	if err := ValidateManagedCodexPrincipal(auth, a[IdentityUserAttribute], a[IdentityAccountAttribute]); err != nil {
		return nil, err
	}
	return p, nil
}

// ApplyManagedIdentity uses exact schema paths only. Unknown metadata, tools,
// encrypted history and existing session identifiers retain their original bytes.
func ApplyManagedIdentity(auth *coreauth.Auth, headers http.Header, body []byte) ([]byte, error) {
	profile, err := ManagedIdentityForAuth(auth)
	if err != nil || profile == nil {
		return body, err
	}
	if !strings.EqualFold(auth.Provider, "codex") {
		return body, nil
	}
	if headers != nil {
		stripCodexCookies(headers)
		for name := range headers {
			if strings.EqualFold(name, "X-Codex-Installation-Id") {
				delete(headers, name)
			}
		}
		headers.Set("X-Codex-Installation-Id", profile.InstallationID)
		for name, values := range headers {
			if !strings.EqualFold(name, "X-Codex-Turn-Metadata") {
				continue
			}
			for i, raw := range values {
				values[i] = rewriteInstallationObject(raw, profile.InstallationID, false, false)
			}
		}
	}
	return []byte(rewriteInstallationObject(string(body), profile.InstallationID, true, true)), nil
}

// Replace only the known schema's values, including duplicate known keys. This
// leaves every byte outside those values (especially encrypted history) intact.
func rewriteInstallationObject(raw, installation string, containers, turnMetadata bool) string {
	if !gjson.Valid(raw) || !gjson.Parse(raw).IsObject() {
		return raw
	}
	type replacement struct {
		start, end int
		value      string
	}
	var replacements []replacement
	encoded, _ := json.Marshal(installation)
	gjson.Parse(raw).ForEach(func(key, value gjson.Result) bool {
		updated := value.Raw
		switch {
		case key.String() == "installation_id" || key.String() == "x-codex-installation-id":
			updated = string(encoded)
		case containers && key.String() == "client_metadata" && value.IsObject():
			updated = rewriteInstallationObject(value.Raw, installation, false, true)
		case turnMetadata && key.String() == "x-codex-turn-metadata" && value.Type == gjson.String:
			rewritten := rewriteInstallationObject(value.String(), installation, false, false)
			if rewritten != value.String() {
				b, _ := json.Marshal(rewritten)
				updated = string(b)
			}
		}
		if updated != value.Raw {
			replacements = append(replacements, replacement{value.Index, value.Index + len(value.Raw), updated})
		}
		return true
	})
	sort.Slice(replacements, func(i, j int) bool { return replacements[i].start > replacements[j].start })
	for _, r := range replacements {
		raw = raw[:r.start] + r.value + raw[r.end:]
	}
	return raw
}

// ApplyManagedIdentityRequest also supports generic JSON and multipart requests.
// Upload bodies are never parsed or decorated as JSON.
func ApplyManagedIdentityRequest(req *http.Request, auth *coreauth.Auth) error {
	profile, err := ManagedIdentityForAuth(auth)
	if err != nil || profile == nil {
		return err
	}
	if req == nil || req.URL == nil {
		return fmt.Errorf("managed identity request is nil")
	}
	if req.Header == nil {
		req.Header = make(http.Header)
	}
	if strings.EqualFold(auth.Provider, "codex") {
		stripCodexCookies(req.Header)
	}
	if !ManagedIdentityProtocolApplies(auth, req.URL.String()) {
		stripCodexInstallationHeader(req.Header)
		return nil
	}
	if _, err = ApplyManagedIdentity(auth, req.Header, nil); err != nil {
		return err
	}
	mime := strings.ToLower(strings.Split(req.Header.Get("Content-Type"), ";")[0])
	if req.Body == nil || (mime != "application/json" && !strings.HasSuffix(mime, "+json")) || !strings.EqualFold(auth.Provider, "codex") {
		return nil
	}
	const limit = 64 << 20
	reader := req.Body
	if req.GetBody != nil {
		reader, err = req.GetBody()
		if err != nil {
			return err
		}
	}
	data, err := io.ReadAll(io.LimitReader(reader, limit+1))
	_ = reader.Close()
	if err != nil {
		return err
	}
	if len(data) > limit {
		return IdentityError{}
	}
	updated, err := ApplyManagedIdentity(auth, nil, data)
	if err != nil {
		return err
	}
	if req.GetBody != nil {
		_ = req.Body.Close()
	}
	req.Body = io.NopCloser(bytes.NewReader(updated))
	req.GetBody = func() (io.ReadCloser, error) { return io.NopCloser(bytes.NewReader(updated)), nil }
	req.ContentLength = int64(len(updated))
	req.Header.Del("Content-Length")
	return nil
}

// ValidateManagedCodexPrincipal is a consistency check, not JWT verification.
// Unknown/opaque claims are allowed; explicitly conflicting claims cannot reuse
// the host's old installation scope after watcher or executor token refreshes.
func ValidateManagedCodexPrincipal(auth *coreauth.Auth, user, account string) error {
	if auth == nil {
		return IdentityError{}
	}
	if !strings.EqualFold(auth.Provider, "codex") || (user == "" && account == "") {
		return nil
	}
	check := func(values map[string]any) error {
		for field, want := range map[string]string{"chatgpt_account_id": account, "chatgpt_user_id": user} {
			if want == "" {
				continue
			}
			aliases := []string{field}
			if field == "chatgpt_account_id" {
				aliases = append(aliases, "account_id")
			} else {
				aliases = append(aliases, "user_id")
			}
			for _, key := range aliases {
				if value, ok := values[key].(string); ok && strings.TrimSpace(value) != "" {
					if value != want {
						return IdentityError{}
					}
					break
				}
			}
		}
		return nil
	}
	if err := check(auth.Metadata); err != nil {
		return err
	}
	// Match codexCreds: only the selected Bearer can establish JWT ownership.
	// Historical ID tokens and unused access-token aliases may describe an old workspace.
	token := auth.Attributes["api_key"]
	if token == "" {
		token, _ = auth.Metadata["access_token"].(string)
	}
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return nil
	}
	payload, err := base64.RawURLEncoding.DecodeString(strings.TrimRight(parts[1], "="))
	if err != nil {
		return nil
	}
	var claims map[string]any
	if json.Unmarshal(payload, &claims) != nil {
		return nil
	}
	if err := check(claims); err != nil {
		return err
	}
	if nested, ok := claims["https://api.openai.com/auth"].(map[string]any); ok {
		if err := check(nested); err != nil {
			return err
		}
	}
	return nil
}

// Standalone API-key image/audio endpoints use their provider's schema. Codex
// OAuth and Responses requests share the host installation profile.
func ManagedIdentityProtocolApplies(auth *coreauth.Auth, endpoint string) bool {
	if auth == nil || !strings.EqualFold(auth.Provider, "codex") {
		return false
	}
	if strings.TrimSpace(auth.Attributes["api_key"]) == "" && auth.AuthKind() != coreauth.AuthKindAPIKey {
		return true
	}
	target, err := url.Parse(endpoint)
	if err != nil {
		return false
	}
	path := strings.TrimRight(target.Path, "/")
	for _, suffix := range []string{"/images/generations", "/images/edits", "/images/variations", "/audio/speech", "/audio/transcriptions", "/audio/translations", "/transcribe"} {
		if strings.HasSuffix(path, suffix) {
			return false
		}
	}
	return true
}

// Standalone image/audio APIs cannot inherit a caller's Codex installation
// header. Cookie credentials also cannot follow a different selected account.
func StripManagedStandaloneIdentityHeaders(auth *coreauth.Auth, headers http.Header) error {
	profile, err := ManagedIdentityForAuth(auth)
	if err != nil {
		return err
	}
	if profile != nil {
		stripCodexInstallationHeader(headers)
		if strings.EqualFold(auth.Provider, "codex") {
			stripCodexCookies(headers)
		}
	}
	return nil
}
func stripCodexInstallationHeader(headers http.Header) {
	for name := range headers {
		if strings.EqualFold(name, "X-Codex-Installation-Id") {
			delete(headers, name)
		}
	}
}
func stripCodexCookies(headers http.Header) {
	for name := range headers {
		if strings.EqualFold(name, "Cookie") || strings.EqualFold(name, "Cookie2") {
			delete(headers, name)
		}
	}
}
