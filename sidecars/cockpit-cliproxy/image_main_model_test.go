package main

import (
	"bytes"
	"context"
	"encoding/json"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestImageMainModelRelayEndpoints(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, setting := range []struct{ name, configured, expected string }{
		{"custom", " gpt-custom-image-main ", "gpt-custom-image-main"},
		{"default", "", "gpt-5.6-luna"},
		{"legacy-manifest", " GPT-5.5 ", "gpt-5.6-luna"},
	} {
		for _, route := range []string{"generate", "edit-json", "edit-multipart"} {
			t.Run(setting.name+"/"+route, func(t *testing.T) {
				chunks := make(chan cliproxyexecutor.StreamChunk, 1)
				chunks <- cliproxyexecutor.StreamChunk{Payload: []byte("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"image_generation_call\",\"result\":\"ZmFrZS1wbmc=\"}]}}\n\n")}
				close(chunks)
				runtime := &fakeRuntime{streamResult: &cliproxyexecutor.StreamResult{Chunks: chunks}}
				key := &apiKeySpec{ID: "key", Key: "client-key", Enabled: true, AllowedModels: []string{"gpt-image-2"}}
				m := &manifest{ModelIDs: []string{"gpt-image-2"}, ImageGenerationModel: "gpt-image-2", ImageGenerationMainModel: setting.configured, apiKeyByValue: map[string]*apiKeySpec{"client-key": key}}
				expected := setting.expected
				router := (&relayServer{runtime: runtime, cfg: &config.Config{}, manifest: m, policy: &requestPolicy{manifest: m}}).router()
				path := "/v1/images/generations"
				contentType := "application/json"
				body := []byte(`{"model":"gpt-image-2","prompt":"draw"}`)
				if route == "edit-json" {
					path = "/v1/images/edits"
					body = []byte(`{"model":"gpt-image-2","prompt":"edit","images":[{"image_url":"data:image/png;base64,ZmFrZQ=="}]}`)
				}
				if route == "edit-multipart" {
					path = "/v1/images/edits"
					var buffer bytes.Buffer
					writer := multipart.NewWriter(&buffer)
					if err := writer.WriteField("model", "gpt-image-2"); err != nil {
						t.Fatal(err)
					}
					if err := writer.WriteField("prompt", "edit"); err != nil {
						t.Fatal(err)
					}
					image, err := writer.CreateFormFile("image", "test.png")
					if err != nil {
						t.Fatal(err)
					}
					if _, err := image.Write([]byte("test image")); err != nil {
						t.Fatal(err)
					}
					if err := writer.Close(); err != nil {
						t.Fatal(err)
					}
					contentType = writer.FormDataContentType()
					body = buffer.Bytes()
				}
				req := httptest.NewRequest(http.MethodPost, path, bytes.NewReader(body))
				req.Header.Set("Authorization", "Bearer client-key")
				req.Header.Set("Content-Type", contentType)
				response := httptest.NewRecorder()
				router.ServeHTTP(response, req)
				if response.Code != http.StatusOK {
					t.Fatalf("status %d: %s", response.Code, response.Body.String())
				}
				if runtime.lastReq.Model != expected {
					t.Fatalf("scheduler model %q, want %q", runtime.lastReq.Model, expected)
				}
				var payload map[string]any
				if err := json.Unmarshal(runtime.lastReq.Payload, &payload); err != nil {
					t.Fatal(err)
				}
				if payload["model"] != expected {
					t.Fatalf("payload model differs: %#v", payload["model"])
				}
				tools := payload["tools"].([]any)
				if tools[0].(map[string]any)["model"] != "gpt-image-2" {
					t.Fatalf("main model changed image tool: %#v", tools)
				}
			})
		}
	}
}

func TestImageMainModelManifestMigratesOnlyLegacyDefault(t *testing.T) {
	for _, tc := range []struct{ input, stored, effective string }{
		{"", "", "gpt-5.6-luna"},
		{" \t ", "", "gpt-5.6-luna"},
		{"gpt-5.5", "gpt-5.6-luna", "gpt-5.6-luna"},
		{" \tGpT-5.5\n", "gpt-5.6-luna", "gpt-5.6-luna"},
		{" gpt-5.4-mini ", "gpt-5.4-mini", "gpt-5.4-mini"},
		{" gpt-5.5-custom ", "gpt-5.5-custom", "gpt-5.5-custom"},
	} {
		t.Run(tc.input, func(t *testing.T) {
			original := manifest{ModelIDs: []string{"gpt-5.5", "gpt-image-2.5"}, ImageGenerationModel: "gpt-image-2.5", ImageGenerationMainModel: tc.input}
			if got := configuredImagesMainModel(&original); got != tc.effective {
				t.Fatalf("effective model before load = %q, want %q", got, tc.effective)
			}
			encoded, err := json.Marshal(&original)
			if err != nil {
				t.Fatal(err)
			}
			path := filepath.Join(t.TempDir(), "manifest.json")
			if err = os.WriteFile(path, encoded, 0600); err != nil {
				t.Fatal(err)
			}
			loaded, err := loadManifest(path)
			if err != nil {
				t.Fatal(err)
			}
			if loaded.ImageGenerationMainModel != tc.stored || configuredImagesMainModel(loaded) != tc.effective {
				t.Fatalf("stored/effective = %q/%q, want %q/%q", loaded.ImageGenerationMainModel, configuredImagesMainModel(loaded), tc.stored, tc.effective)
			}
			if len(loaded.ModelIDs) != 2 || loaded.ModelIDs[0] != "gpt-5.5" || loaded.ImageGenerationModel != "gpt-image-2.5" {
				t.Fatalf("unrelated models changed: %#v / %q", loaded.ModelIDs, loaded.ImageGenerationModel)
			}
			if strings.EqualFold(strings.TrimSpace(tc.input), "gpt-5.5") {
				found := false
				for _, model := range manifestModelsForAuth(&original, &coreauth.Auth{ID: "oauth", Provider: "codex"}) {
					if model.ID == tc.effective {
						found = true
					}
				}
				if !found {
					t.Fatal("scheduler did not register the effective replacement for an old manifest")
				}
			}
		})
	}
}

func TestImageMainModelRegistrationIsInternalAndHonorsExclusions(t *testing.T) {
	main := "gpt-custom-image-main"
	m := &manifest{ModelIDs: []string{"gpt-image-2"}, ImageGenerationMainModel: main}
	auth := &coreauth.Auth{ID: "oauth", Provider: "codex"}
	count := func() int {
		n := 0
		for _, model := range manifestModelsForAuth(m, auth) {
			if model != nil && strings.EqualFold(model.ID, main) {
				n++
			}
		}
		return n
	}
	if count() != 1 {
		t.Fatal("explicit main model must be registered internally")
	}
	for _, visible := range visibleModelsForAPIKey(m, nil) {
		if visible == main {
			t.Fatal("image-only model leaked into text catalog")
		}
	}
	key := &apiKeySpec{AllowedModels: []string{"gpt-image-2"}}
	if validateClientModelVisible(m, key, main, main) {
		t.Fatal("image setting widened API key text permissions")
	}
	models := filterRegistryModelsByExcluded(manifestModelsForAuth(m, auth), []string{main})
	for _, model := range models {
		if model.ID == main {
			t.Fatal("explicit exclusions must still apply")
		}
	}
	m.ModelIDs = append(m.ModelIDs, strings.ToUpper(main))
	if count() != 1 { // Existing case-insensitive ID is retained, never duplicated.
		t.Fatal("explicit setting duplicated an existing model")
	}
	m.ImageGenerationMainModel = ""
	m.ModelIDs = []string{"gpt-image-2"}
	auth.Attributes = map[string]string{coreauth.AttributeAuthKind: coreauth.AuthKindAPIKey}
	for _, model := range manifestModelsForAuth(m, auth) {
		if model.ID == defaultImagesMainModel {
			t.Fatal("OAuth image default must not expand API key scheduler catalog")
		}
	}
	if configuredImagesMainModel(nil) != defaultImagesMainModel {
		t.Fatal("nil manifest changed legacy model")
	}
}

func TestImageMainModelDefaultRegistersForOAuthSchedulerOnly(t *testing.T) {
	for _, setting := range []struct{ name, value string }{{"missing", ""}, {"blank", " \t "}} {
		t.Run(setting.name, func(t *testing.T) {
			m := &manifest{ModelIDs: []string{"vendor-chat", "gpt-image-2"}, ImageGenerationMainModel: setting.value}
			auth := &coreauth.Auth{
				ID: "image-main-default-" + setting.name, Provider: "codex", Status: coreauth.StatusActive,
				Attributes: map[string]string{coreauth.AttributeAuthKind: coreauth.AuthKindOAuth},
			}
			manager := coreauth.NewManager(nil, &coreauth.RoundRobinSelector{}, nil)
			manager.RegisterExecutor(executor.NewCodexExecutor(&config.Config{}))
			if _, err := manager.Register(context.Background(), auth); err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { cliproxy.GlobalModelRegistry().UnregisterClient(auth.ID) })
			main := configuredImagesMainModel(m)
			if _, err := manager.SelectAuthByKind(context.Background(), "codex", main, coreauth.AuthKindOAuth, cliproxyexecutor.Options{}); err == nil {
				t.Fatal("scheduler accepted an unregistered model")
			}
			registerManifestModelsForAuth(manager, m, auth)
			selected, err := manager.SelectAuthByKind(context.Background(), "codex", main, coreauth.AuthKindOAuth, cliproxyexecutor.Options{})
			if err != nil || selected == nil || selected.ID != auth.ID {
				t.Fatalf("default image main model not selectable: selected=%v err=%v", selected, err)
			}
			key := &apiKeySpec{AllowedModels: []string{"gpt-image-2"}}
			for _, spec := range []*apiKeySpec{nil, key} {
				for _, visible := range visibleModelsForAPIKey(m, spec) {
					if strings.EqualFold(visible, main) {
						t.Fatal("internal image default leaked into client catalog")
					}
				}
				if validateClientModelVisible(m, spec, main, main) {
					t.Fatal("internal image default widened text permissions")
				}
			}
			m.ExcludedModels = []string{main}
			registerManifestModelsForAuth(manager, m, auth)
			if _, err := manager.SelectAuthByKind(context.Background(), "codex", main, coreauth.AuthKindOAuth, cliproxyexecutor.Options{}); err == nil {
				t.Fatal("default image main model bypassed explicit model exclusions")
			}
		})
	}
}

func TestImageMainModelLegacyDefaultDoesNotRewriteOrdinaryConversation(t *testing.T) {
	gin.SetMode(gin.TestMode)
	runtime := &fakeRuntime{response: cliproxyexecutor.Response{Payload: []byte(`{"ok":true}`)}}
	key := &apiKeySpec{ID: "key", Key: "client-key", Enabled: true, AllowedModels: []string{"gpt-5.6-sol"}}
	m := &manifest{
		ModelIDs: []string{"gpt-5.6-sol"}, ImageGenerationModel: "gpt-image-2.5",
		ImageGenerationMainModel: " GPT-5.5 ", apiKeyByValue: map[string]*apiKeySpec{"client-key": key},
	}
	router := (&relayServer{runtime: runtime, cfg: &config.Config{}, manifest: m, policy: &requestPolicy{manifest: m}}).router()
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.6-sol","input":"ordinary conversation","stream":false}`))
	request.Header.Set("Authorization", "Bearer client-key")
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("status %d: %s", response.Code, response.Body.String())
	}
	var payload map[string]any
	if err := json.Unmarshal(runtime.lastReq.Payload, &payload); err != nil {
		t.Fatal(err)
	}
	if runtime.lastReq.Model != "gpt-5.6-sol" || payload["model"] != "gpt-5.6-sol" {
		t.Fatalf("image default rewrote conversation model: %q / %#v", runtime.lastReq.Model, payload["model"])
	}
}
