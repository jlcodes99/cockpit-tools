package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func gatewayIdentityFixture() json.RawMessage {
	return json.RawMessage(`{"version":1,"scope":"` + strings.Repeat("a", 64) + `","installationId":"11111111-1111-4111-8111-111111111111","revision":1}`)
}
func TestGatewayIdentityManifestAndTokenProjection(t *testing.T) {
	for _, raw := range []string{"", string(gatewayIdentityFixture()), "null", `{"version":1}`} {
		payload := `{"accounts":[{"id":"a","authId":"a.json"` + map[bool]string{true: `,"gatewayIdentity":` + raw, false: ""}[raw != ""] + `}]}`
		path := filepath.Join(t.TempDir(), "manifest.json")
		if err := os.WriteFile(path, []byte(payload), 0600); err != nil {
			t.Fatal(err)
		}
		_, err := loadManifest(path)
		if (raw == "" || raw == string(gatewayIdentityFixture())) != (err == nil) {
			t.Fatalf("profile %s: %v", raw, err)
		}
	}
	dir := t.TempDir()
	path := filepath.Join(dir, "a.json")
	if err := os.WriteFile(path, []byte(`{"type":"codex","access_token":"fixture","account_id":"team-a"}`), 0600); err != nil {
		t.Fatal(err)
	}
	account := &accountSpec{ID: "a", AuthID: "a.json", ChatGPTAccountID: "team-a", GatewayIdentity: gatewayIdentityFixture()}
	a, err := readManifestCodexTokenAuth(account, dir, path)
	if err != nil {
		t.Fatal(err)
	}
	if a.Attributes[helps.InstallationIDAttribute] != "11111111-1111-4111-8111-111111111111" || a.Attributes["account_id"] != "a" || a.Attributes["chatgpt_account_id"] != "team-a" {
		t.Fatal("identity projection missing")
	}
	rt := newSidecarRoundTripperProvider()
	first := rt.RoundTripperFor(a)
	if first == nil || first != rt.RoundTripperFor(a) {
		t.Fatal("pool not retained")
	}
	b := a.Clone()
	b.Attributes["account_id"] = "b"
	if first == rt.RoundTripperFor(b) {
		t.Fatal("account pools shared")
	}
	b = a.Clone()
	b.Metadata["access_token"] = "refreshed"
	if first == rt.RoundTripperFor(b) {
		t.Fatal("credential pool not retired")
	}
	legacy := &coreauth.Auth{}
	if rt.RoundTripperFor(legacy) != nil {
		t.Fatal("legacy direct transport changed")
	}
}
func TestGatewayIdentityDirectProviderAndAudioCallPaths(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer fixture" {
			t.Error("credential changed")
		}
		body, _ := io.ReadAll(r.Body)
		if r.URL.Path == "/v1/responses" {
			if r.Header.Get("X-Codex-Installation-Id") != "11111111-1111-4111-8111-111111111111" || gjson.GetBytes(body, "installation_id").String() != "11111111-1111-4111-8111-111111111111" {
				t.Error("Responses identity missing")
			}
		} else {
			if r.Header.Get("X-Codex-Installation-Id") != "" {
				t.Error("standalone API received identity")
			}
			if r.URL.Path == "/v1/audio/speech" && gjson.GetBytes(body, "metadata.installation_id").String() != "business" {
				t.Error("audio body changed")
			}
		}
		if r.URL.Path == "/v1/chat/completions" {
			_, _ = io.WriteString(w, `{"id":"chat","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}`)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"id":"r","object":"response","status":"completed","output":[]}`)
	}))
	defer upstream.Close()
	account := accountSpec{ID: "a", UpstreamAPIKey: "fixture", GatewayIdentity: gatewayIdentityFixture()}
	s := &relayServer{manifest: &manifest{Accounts: []accountSpec{account}, accountByID: map[string]*accountSpec{"a": &account}}}
	gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "fixture", WireAPI: "responses", UpstreamModel: "test", UpstreamModels: []string{"test"}}
	body := []byte(`{"model":"test","input":"hello","installation_id":"client","metadata":{"installation_id":"business"}}`)
	makeContext := func() (*gin.Context, *httptest.ResponseRecorder) {
		w := httptest.NewRecorder()
		c, _ := gin.CreateTestContext(w)
		c.Request = httptest.NewRequest("POST", "/v1/responses", nil).WithContext(context.WithValue(context.Background(), providerGatewayAccountContextKey{}, "a"))
		return c, w
	}
	c, w := makeContext()
	s.handleProviderGatewayRequest(c, gateway, body, "test", sdktranslator.FormatOpenAIResponse, "")
	if w.Code != 200 {
		t.Fatal(w.Code, w.Body.String())
	}
	gateway.WireAPI = "chat_completions"
	c, w = makeContext()
	s.handleProviderGatewayRequest(c, gateway, body, "test", sdktranslator.FormatOpenAIResponse, "")
	if w.Code != 200 {
		t.Fatal(w.Code, w.Body.String())
	}
	c, _ = makeContext()
	resp, _, err := s.forwardProviderAudio(c, gateway, &audioRequest{body: body, contentType: "application/json", operation: "speech"}, "test")
	if err != nil {
		t.Fatal(err)
	}
	if resp == nil {
		t.Fatal("no audio response")
	}
	_, _ = io.Copy(io.Discard, resp.Body)
	_ = resp.Body.Close()
}
func TestGatewayIdentityImageCallPathAndAnonymousDownload(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "GET" {
			if r.Header.Get("Authorization") != "" || r.Header.Get("X-Codex-Installation-Id") != "" {
				t.Error("anonymous download received account identity")
			}
			_, _ = w.Write(imageAPITestPNG(t))
			return
		}
		if r.Header.Get("Authorization") != "Bearer upstream-secret" || r.Header.Get("X-Codex-Installation-Id") != "" {
			t.Error("third-party image headers changed")
		}
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"`+imageAPITestPNGBase64+`"}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	s.manifest.Accounts[0].GatewayIdentity = gatewayIdentityFixture()
	w := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if w.Code != 200 {
		t.Fatal(w.Code, w.Body.String())
	}
	client, err := imageDownloadClient(&imageAPIUpstreamSpec{}, imageAPITestDownloadPolicy(t, upstream.URL))
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := imageAPIUpload(context.Background(), client, "http://images.test/image"); err != nil {
		t.Fatal(err)
	}
}

func TestGatewayIdentityStrictOwnershipAndChangedClaims(t *testing.T) {
	account := accountSpec{ID: "a", AuthID: "a.json", Email: "same@example.invalid", ChatGPTUserID: "user-a", ChatGPTAccountID: "team-a", GatewayIdentity: gatewayIdentityFixture()}
	m := &manifest{Accounts: []accountSpec{account}, accountByEmail: map[string]*accountSpec{account.Email: &account}}
	a := &coreauth.Auth{ID: "a.json", Provider: "codex", Attributes: map[string]string{}, Metadata: map[string]any{"access_token": "first", "account_id": "team-a", "chatgpt_user_id": "user-a"}}
	if err := bindGatewayIdentityForManifestAuth(m, a); err != nil {
		t.Fatal(err)
	}
	a.Metadata["access_token"] = "normal-refresh"
	if _, err := helps.ManagedIdentityForAuth(a); err != nil {
		t.Fatal(err)
	}
	a.Metadata["chatgpt_user_id"] = "user-other"
	if _, err := helps.ManagedIdentityForAuth(a); err == nil {
		t.Fatal("changed principal retained scope")
	}
	a.Metadata["chatgpt_user_id"] = "user-a"
	a.Metadata["account_id"] = "team-other"
	if err := bindGatewayIdentityForManifestAuth(m, a); err == nil {
		t.Fatal("changed workspace accepted")
	}
	unrelated := &coreauth.Auth{ID: "other.json", Provider: "codex", Attributes: map[string]string{"email": account.Email}}
	if err := bindGatewayIdentityForManifestAuth(m, unrelated); err == nil {
		t.Fatal("email match adopted identity")
	}
	m.Accounts[0].UpstreamAPIKey = "fixture"
	duplicate := account
	duplicate.UpstreamAPIKey = "fixture"
	duplicate.ID = "b"
	duplicate.AuthID = "b.json"
	m.Accounts = append(m.Accounts, duplicate)
	keyAuth := &coreauth.Auth{ID: "synth", Provider: "codex", Attributes: map[string]string{"api_key": "fixture"}}
	if err := bindGatewayIdentityForManifestAuth(m, keyAuth); err == nil {
		t.Fatal("ambiguous key adopted identity")
	}
}

func TestGatewayIdentityMissingOwnerCannotOptOut(t *testing.T) {
	account := accountSpec{ID: "a", UpstreamAPIKey: "key-a", GatewayIdentity: gatewayIdentityFixture()}
	m := &manifest{Accounts: []accountSpec{account}, accountByID: map[string]*accountSpec{"a": &account}}
	incoming := &coreauth.Auth{ID: "unknown", Provider: "codex", Attributes: map[string]string{"api_key": "new-unbound-key"}}
	if err := bindGatewayIdentityForManifestAuth(m, incoming); err == nil {
		t.Fatal("unbound auth escaped managed pool")
	}
	s := &relayServer{manifest: m}
	if _, err := s.directGatewayIdentity("a", "other-key", "http://example.invalid", true); err == nil {
		t.Fatal("wrong account credential accepted")
	}
	if _, err := s.directGatewayIdentity("missing", "other-key", "http://example.invalid", true); err == nil {
		t.Fatal("missing account escaped managed pool")
	}
	account.GatewayIdentity = nil
	m.Accounts[0].GatewayIdentity = nil
	if err := bindGatewayIdentityForManifestAuth(m, incoming); err != nil {
		t.Fatal("legacy behavior changed", err)
	}
}

func TestGatewayIdentityInitialRuntimeLoadIsProjected(t *testing.T) {
	cfg := &config.Config{AuthDir: t.TempDir()}
	manager := coreauth.NewManager(nil, nil, nil)
	auth := &coreauth.Auth{ID: "stored", Provider: "codex", Attributes: map[string]string{"api_key": "fixture"}}
	if _, err := manager.Register(context.Background(), auth); err != nil {
		t.Fatal(err)
	}
	account := accountSpec{ID: "a", AuthKind: "api_key", UpstreamAPIKey: "fixture", GatewayIdentity: gatewayIdentityFixture()}
	m := &manifest{Accounts: []accountSpec{account}, accountByAuthID: map[string]*accountSpec{}}
	service, err := cliproxy.NewBuilder().WithConfig(cfg).WithConfigPath(filepath.Join(cfg.AuthDir, "fixture-config.yaml")).WithCoreAuthManager(manager).Build()
	if err != nil {
		t.Fatal(err)
	}
	if err := initializeSidecarRuntimeAuths(context.Background(), service, cfg, m, manager); err != nil {
		t.Fatal(err)
	}
	current, _ := manager.GetByID("stored")
	if current.Attributes[helps.IdentityScopeAttribute] == "" {
		t.Fatal("initially loaded auth remained unmanaged")
	}
	if _, err := manager.Register(context.Background(), &coreauth.Auth{ID: "unknown", Provider: "codex", Attributes: map[string]string{"api_key": "other"}}); err != nil {
		t.Fatal(err)
	}
	if err := initializeSidecarRuntimeAuths(context.Background(), service, cfg, m, manager); err == nil {
		t.Fatal("unbound initial auth accepted")
	}
}

func TestGatewayIdentityManifestRejectsDuplicateProfiles(t *testing.T) {
	for _, kind := range []string{"account", "scope", "installation"} {
		first := accountSpec{ID: "a", GatewayIdentity: gatewayIdentityFixture()}
		second := accountSpec{ID: "b", GatewayIdentity: json.RawMessage(`{"version":1,"scope":"` + strings.Repeat("b", 64) + `","installationId":"22222222-2222-4222-8222-222222222222","revision":1}`)}
		switch kind {
		case "account":
			second.ID = first.ID
		case "scope":
			second.GatewayIdentity = json.RawMessage(strings.Replace(string(second.GatewayIdentity), strings.Repeat("b", 64), strings.Repeat("a", 64), 1))
		case "installation":
			second.GatewayIdentity = json.RawMessage(strings.Replace(string(second.GatewayIdentity), "22222222-2222-4222-8222-222222222222", "11111111-1111-4111-8111-111111111111", 1))
		}
		payload, err := json.Marshal(manifest{Accounts: []accountSpec{first, second}})
		if err != nil {
			t.Fatal(err)
		}
		path := filepath.Join(t.TempDir(), "manifest.json")
		if err := os.WriteFile(path, payload, 0600); err != nil {
			t.Fatal(err)
		}
		if _, err := loadManifest(path); err == nil {
			t.Fatal("accepted duplicate", kind)
		}
	}
}
