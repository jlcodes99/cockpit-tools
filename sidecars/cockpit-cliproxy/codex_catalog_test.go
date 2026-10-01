package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	internalconfig "github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/modelconfig"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/thinking"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func hostCodexCatalogFixture(t *testing.T) []byte {
	t.Helper()
	original := registry.GetCodexClientModelsJSON()
	t.Cleanup(func() {
		if _, err := registry.LoadCodexClientModelsJSON(original); err != nil {
			t.Fatalf("restore catalog: %v", err)
		}
	})
	var document map[string]any
	if err := json.Unmarshal(original, &document); err != nil {
		t.Fatal(err)
	}
	models := document["models"].([]any)
	var future map[string]any
	for _, raw := range models {
		model := raw.(map[string]any)
		if model["slug"] == "gpt-5.5" {
			future = make(map[string]any, len(model))
			for key, value := range model {
				future[key] = value
			}
		}
		if model["slug"] == "gpt-6-sol" {
			model["supported_reasoning_levels"] = []any{map[string]any{"effort": "medium"}, map[string]any{"effort": "max"}}
			model["default_reasoning_level"] = "max"
		}
	}
	if future == nil {
		t.Fatal("missing gpt-5.5 fixture template")
	}
	future["slug"] = "gpt-6.2-sol"
	future["display_name"] = "GPT-6.2 Sol Preview"
	future["context_window"] = 412000
	future["max_context_window"] = 824000
	future["auto_compact_token_limit"] = 370800
	future["supported_reasoning_levels"] = []any{map[string]any{"effort": "low"}, map[string]any{"effort": "ultra"}}
	future["default_reasoning_level"] = "ultra"
	document["models"] = append(models, future)
	data, err := json.Marshal(document)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func writeHostCodexCatalog(t *testing.T, data []byte) *manifest {
	t.Helper()
	path := filepath.Join(t.TempDir(), "catalog.json")
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	return &manifest{CodexClientModelsPath: path, CodexClientModelsHash: fmt.Sprintf("%x", sha256.Sum256(data))}
}

func TestLoadManifestCodexClientModels(t *testing.T) {
	data := hostCodexCatalogFixture(t)
	good := writeHostCodexCatalog(t, data)
	if err := loadManifestCodexClientModels(good); err != nil {
		t.Fatal(err)
	}
	if !good.codexClientModelsLoaded || !bytes.Equal(registry.GetCodexClientModelsJSON(), data) {
		t.Fatal("loader must accept the exact host snapshot")
	}
	before, revision := registry.GetCodexClientModelsSnapshot()
	for _, tc := range []struct {
		name string
		m    *manifest
		want string
	}{
		{"nil", nil, ""},
		{"omitted", &manifest{}, ""},
		{"missing hash", &manifest{CodexClientModelsPath: good.CodexClientModelsPath}, "codexClientModelsHash"},
		{"missing path", &manifest{CodexClientModelsHash: good.CodexClientModelsHash}, "codexClientModelsPath"},
		{"malformed hash", &manifest{CodexClientModelsPath: good.CodexClientModelsPath, CodexClientModelsHash: "bad"}, "SHA256"},
		{"mismatch", &manifest{CodexClientModelsPath: good.CodexClientModelsPath, CodexClientModelsHash: strings.Repeat("0", 64)}, "mismatch"},
		{"missing file", &manifest{CodexClientModelsPath: good.CodexClientModelsPath + ".missing", CodexClientModelsHash: good.CodexClientModelsHash}, "read"},
		{"invalid data", writeHostCodexCatalog(t, []byte(`{"models":[]}`)), "no models"},
		{"too large", writeHostCodexCatalog(t, bytes.Repeat([]byte(" "), (8<<20)+1)), "8 MiB"},
		{"raw bytes changed", writeHostCodexCatalog(t, append(append([]byte(nil), data...), '\n')), ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if tc.name == "raw bytes changed" {
				tc.m.CodexClientModelsHash = good.CodexClientModelsHash
				tc.want = "mismatch"
			}
			err := loadManifestCodexClientModels(tc.m)
			if tc.want == "" && err != nil || tc.want != "" && (err == nil || !strings.Contains(err.Error(), tc.want)) {
				t.Fatalf("load error = %v, want %q", err, tc.want)
			}
			after, afterRevision := registry.GetCodexClientModelsSnapshot()
			if !bytes.Equal(after, before) || afterRevision != revision {
				t.Fatal("omitted or rejected snapshot replaced the accepted catalog")
			}
			if tc.m != nil && tc.m.codexClientModelsLoaded {
				t.Fatal("omitted or rejected snapshot marked loaded")
			}
		})
	}
	path := filepath.Join(t.TempDir(), "manifest.json")
	encoded, _ := json.Marshal(good)
	if err := os.WriteFile(path, encoded, 0o600); err != nil {
		t.Fatal(err)
	}
	loaded, err := loadManifest(path)
	if err != nil || loaded.CodexClientModelsPath != good.CodexClientModelsPath || loaded.CodexClientModelsHash != good.CodexClientModelsHash {
		t.Fatalf("manifest snapshot fields did not survive decoding: %v", err)
	}
}

func TestHostCodexCatalogRejectsStartupBeforeAuthRegistration(t *testing.T) {
	if os.Getenv("COCKPIT_TEST_BAD_CATALOG_STARTUP") == "1" {
		os.Args = []string{"cockpit-cliproxy", "--config", os.Getenv("COCKPIT_TEST_CONFIG"), "--manifest", os.Getenv("COCKPIT_TEST_MANIFEST")}
		main()
		t.Fatal("bad snapshot startup returned without exit")
	}
	m := writeHostCodexCatalog(t, hostCodexCatalogFixture(t))
	m.CodexClientModelsHash = strings.Repeat("0", 64)
	dir := t.TempDir()
	manifestPath := filepath.Join(dir, "manifest.json")
	configPath := filepath.Join(dir, "config.yaml")
	authDir := filepath.Join(dir, "must-not-be-created")
	encoded, err := json.Marshal(m)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(manifestPath, encoded, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(configPath, []byte(fmt.Sprintf("auth-dir: %q\n", filepath.ToSlash(authDir))), 0o600); err != nil {
		t.Fatal(err)
	}
	command := exec.Command(os.Args[0], "-test.run=^TestHostCodexCatalogRejectsStartupBeforeAuthRegistration$")
	command.Env = append(os.Environ(), "COCKPIT_TEST_BAD_CATALOG_STARTUP=1", "COCKPIT_TEST_CONFIG="+configPath, "COCKPIT_TEST_MANIFEST="+manifestPath)
	output, err := command.CombinedOutput()
	var exitError *exec.ExitError
	if !errors.As(err, &exitError) || exitError.ExitCode() != 2 {
		t.Fatalf("startup exit = %v, output=%s", err, output)
	}
	if !bytes.Contains(output, []byte(`"type":"error"`)) || !bytes.Contains(output, []byte("SHA256 mismatch")) || bytes.Contains(output, []byte(`"stage":"init_runtime"`)) || bytes.Contains(output, []byte(`"type":"ready"`)) {
		t.Fatalf("bad snapshot was not rejected before runtime initialization: %s", output)
	}
	if _, err := os.Stat(authDir); !os.IsNotExist(err) {
		t.Fatalf("runtime initialized auth directory before validating snapshot: %v", err)
	}
}

func TestHostCodexCatalogRegistrationAndMetadata(t *testing.T) {
	m := writeHostCodexCatalog(t, hostCodexCatalogFixture(t))
	if err := loadManifestCodexClientModels(m); err != nil {
		t.Fatal(err)
	}
	m.ModelIDs = []string{"gpt-6.2-sol", "gpt-6-sol", "gpt-5.5"}
	m.ModelAliases = []modelAliasSpec{{SourceModel: "gpt-6.2-sol", Alias: "preview-alias"}}
	models := manifestRegistryModels(m)
	for _, id := range []string{"gpt-6.2-sol", "preview-alias"} {
		info := findModelInfoForTest(models, id)
		if info == nil || info.Thinking == nil || !reflect.DeepEqual(info.Thinking.Levels, []string{"low", "ultra"}) || info.UserDefined {
			t.Fatalf("catalog-only %s capabilities = %#v", id, info)
		}
		if info.ContextLength != 412000 {
			t.Fatalf("%s context = %d, want 412000", id, info.ContextLength)
		}
	}
	known := findModelInfoForTest(models, "gpt-6-sol")
	if known == nil || known.Thinking == nil || !reflect.DeepEqual(known.Thinking.Levels, []string{"medium", "max"}) {
		t.Fatalf("stale static reasoning took priority: %#v", known)
	}
	legacy := findModelInfoForTest(manifestRegistryModels(&manifest{ModelIDs: []string{"gpt-6-sol"}}), "gpt-6-sol")
	static := registry.LookupStaticModelInfo("gpt-6-sol")
	if legacy == nil || static == nil || !reflect.DeepEqual(legacy.Thinking, static.Thinking) {
		t.Fatal("manifest without host snapshot must retain prior static fallback")
	}
	auth := &coreauth.Auth{ID: "host-catalog-test", Provider: "codex", Status: coreauth.StatusActive}
	manager := buildCoreAuthManager(&config.Config{}, &cockpitSelector{}, nil, nil, nil, nil)
	if _, err := manager.Register(context.Background(), auth); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(auth.ID) })
	m.ExcludedModels = []string{"gpt-5.5"}
	registerManifestModelsForAuth(manager, m, auth)
	registered := registry.GetGlobalRegistry().GetModelsForClient(auth.ID)
	if findModelInfoForTest(registered, "gpt-5.5") != nil || findModelInfoForTest(registered, "gpt-6.2-sol") == nil {
		t.Fatal("snapshot registration ignored manifest exclusions")
	}
	out, err := thinking.ApplyThinking([]byte(`{"model":"gpt-6.2-sol","reasoning":{"effort":"ultra"}}`), "gpt-6.2-sol", "openai-response", "codex", "codex")
	if err != nil || !bytes.Contains(out, []byte(`"ultra"`)) {
		t.Fatalf("catalog-only reasoning did not survive execution pipeline: %s, %v", out, err)
	}
	spec := &apiKeySpec{AllowedModels: []string{"gpt-6.2-sol"}}
	response := buildCodexClientModelsResponse(clientCatalogModelsForAPIKey(m, spec), spec, nil, m)
	assertFutureCodexMetadata(t, response, "GPT-6.2 Sol Preview", "ultra")
	for _, entry := range response["models"].([]map[string]any) {
		if entry["slug"] == "gpt-6-sol" || entry["slug"] == "gpt-5.5" {
			t.Fatal("catalog metadata leaked models outside API-key scope")
		}
	}
	spec.ModelRouting = &modelRoutingSpec{Automatic: true, NativeModels: []string{"gpt-6.2-sol"}, Routes: []modelRouteSpec{{
		ID: "preview-route", Namespace: "api-preview", ProviderGateway: &providerGatewaySpec{},
		Models: []modelRouteModelSpec{{ClientModel: "gpt-6.2-sol", UpstreamModel: "gpt-6.2-sol", DisplayName: "Configured Preview", ReasoningLevels: []any{map[string]any{"effort": "high"}}, DefaultReasoningLevel: "high"}},
	}}}
	response = buildCodexClientModelsResponse(clientCatalogModelsForAPIKey(m, spec), spec, nil, m)
	assertFutureCodexMetadata(t, response, "Configured Preview", "high")
	if _, _, err := rewriteBodyModel(m, spec, "text", []byte(`{"model":"gpt-6.2-sol"}`)); err != nil {
		t.Fatalf("automatic route did not accept future slug: %v", err)
	}
	if _, _, err := rewriteBodyModel(m, spec, "text", []byte(`{"model":"gpt-6-sol"}`)); err == nil {
		t.Fatal("automatic routing exposed a non-native snapshot model")
	}
}

func assertFutureCodexMetadata(t *testing.T, response gin.H, name, defaultEffort string) {
	t.Helper()
	for _, model := range response["models"].([]map[string]any) {
		if model["slug"] != "gpt-6.2-sol" {
			continue
		}
		if model["display_name"] != name || model["default_reasoning_level"] != defaultEffort || intModelValueAny(model["context_window"]) != 412000 || intModelValueAny(model["max_context_window"]) != 824000 || intModelValueAny(model["auto_compact_token_limit"]) != 370800 {
			t.Fatalf("future catalog metadata = %#v", model)
		}
		var efforts []string
		for _, raw := range model["supported_reasoning_levels"].([]any) {
			efforts = append(efforts, raw.(map[string]any)["effort"].(string))
		}
		want := []string{"low", "ultra"}
		if defaultEffort == "high" {
			want = []string{"high"}
		}
		if !reflect.DeepEqual(efforts, want) {
			t.Fatalf("reasoning levels = %v, want %v", efforts, want)
		}
		return
	}
	t.Fatal("future slug missing from response")
}

func TestHostCodexCatalogStartupWithProjectedAPIKeyThinking(t *testing.T) {
	gin.SetMode(gin.TestMode)
	m := writeHostCodexCatalog(t, hostCodexCatalogFixture(t))
	if err := loadManifestCodexClientModels(m); err != nil {
		t.Fatal(err)
	}
	// Configured API-key capabilities are private snapshots, not global catalog lookups.
	withoutProjection := modelconfig.ResolveModelInfo("gpt-6.2-sol", "codex", nil)
	if withoutProjection.Thinking != nil {
		t.Fatal("vendor API-key modelconfig unexpectedly learned catalog-only thinking")
	}
	requests := make(chan map[string]any, 4)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/responses" || r.Header.Get("Authorization") != "Bearer test-upstream-key" {
			t.Errorf("unexpected upstream request: %s %s", r.URL.Path, r.Header.Get("Authorization"))
		}
		body, _ := io.ReadAll(r.Body)
		var payload map[string]any
		if err := json.Unmarshal(body, &payload); err != nil {
			t.Errorf("invalid upstream body: %v", err)
		}
		requests <- payload
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = fmt.Fprintf(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-test\",\"object\":\"response\",\"status\":\"completed\",\"model\":%q,\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n", payload["model"])
	}))
	defer upstream.Close()
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.yaml")
	if err := os.WriteFile(configPath, []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{AuthDir: filepath.Join(tempDir, "auths"), CodexKey: []config.CodexKey{{
		APIKey: "test-upstream-key", BaseURL: upstream.URL,
		Models: []internalconfig.CodexModel{
			{Name: "gpt-6.2-sol", Thinking: &registry.ThinkingSupport{Levels: []string{"low", "ultra"}}},
			{Name: "gpt-6-sol", Thinking: &registry.ThinkingSupport{Levels: []string{"high"}}},
		},
	}}}
	m.ModelIDs = []string{"gpt-6.2-sol", "gpt-6-sol"}
	m.Accounts = []accountSpec{{ID: "test-account", AuthKind: "api_key", UpstreamAPIKey: "test-upstream-key"}}
	m.accountByID = map[string]*accountSpec{"test-account": &m.Accounts[0]}
	m.accountByAPIKey = map[string]*accountSpec{"test-upstream-key": &m.Accounts[0]}
	m.accountByAuthID = map[string]*accountSpec{}
	spec := &apiKeySpec{ID: "test-client", Key: "client-key", Enabled: true, AccountIDs: []string{"test-account"}, AllowedModels: []string{"gpt-6.2-sol", "gpt-6-sol"}}
	m.apiKeyByValue = map[string]*apiKeySpec{"client-key": spec}
	tracker := newRequestUsageTracker()
	selector := &cockpitSelector{manifest: m}
	manager := buildCoreAuthManager(cfg, selector, &authHook{manifest: m}, m, nil, tracker)
	runtime, err := newSidecarRuntime(context.Background(), configPath, cfg, m, manager)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		runtime.Stop()
		for _, auth := range manager.List() {
			registry.GetGlobalRegistry().UnregisterClient(auth.ID)
		}
	})
	if len(manager.List()) != 1 || findModelInfoForTest(registry.GetGlobalRegistry().GetModelsForClient(manager.List()[0].ID), "gpt-6.2-sol") == nil {
		t.Fatal("future slug was not registered at startup")
	}
	router := (&relayServer{runtime: runtime, cfg: cfg, manifest: m, authManager: manager, policy: &requestPolicy{manifest: m, cfg: cfg}}).router()
	for _, tc := range []struct{ model, effort string }{{"gpt-6.2-sol", "ultra"}, {"gpt-6-sol", "high"}} {
		req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(fmt.Sprintf(`{"model":%q,"input":"hello","reasoning":{"effort":%q},"stream":false}`, tc.model, tc.effort)))
		req.Header.Set("Authorization", "Bearer client-key")
		req.Header.Set("Content-Type", "application/json")
		w := httptest.NewRecorder()
		router.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("%s execution status = %d, body=%s", tc.model, w.Code, w.Body.String())
		}
		select {
		case payload := <-requests:
			reasoning, _ := payload["reasoning"].(map[string]any)
			if payload["model"] != tc.model || reasoning["effort"] != tc.effort {
				t.Fatalf("projected/explicit thinking not honored: %#v", payload)
			}
		default:
			t.Fatal("mock upstream was not called")
		}
	}
	if got := cfg.CodexKey[0].Models[1].Thinking.Levels; !reflect.DeepEqual(got, []string{"high"}) {
		t.Fatalf("explicit configured thinking mutated: %v", got)
	}
}
