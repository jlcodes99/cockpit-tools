package main

import (
	"bytes"
	"encoding/json"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestImageMainModelRelayEndpoints(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, route := range []string{"generate", "edit-json", "edit-multipart", "legacy"} {
		t.Run(route, func(t *testing.T) {
			chunks := make(chan cliproxyexecutor.StreamChunk, 1)
			chunks <- cliproxyexecutor.StreamChunk{Payload: []byte("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"image_generation_call\",\"result\":\"ZmFrZS1wbmc=\"}]}}\n\n")}
			close(chunks)
			runtime := &fakeRuntime{streamResult: &cliproxyexecutor.StreamResult{Chunks: chunks}}
			key := &apiKeySpec{ID: "key", Key: "client-key", Enabled: true, AllowedModels: []string{"gpt-image-2"}}
			m := &manifest{ModelIDs: []string{"gpt-image-2"}, ImageGenerationModel: "gpt-image-2", ImageGenerationMainModel: " gpt-custom-image-main ", apiKeyByValue: map[string]*apiKeySpec{"client-key": key}}
			expected := "gpt-custom-image-main"
			if route == "legacy" {
				m.ImageGenerationMainModel = ""
				expected = defaultImagesMainModel
			}
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
	for _, model := range manifestModelsForAuth(m, auth) {
		if model.ID == defaultImagesMainModel {
			t.Fatal("legacy configuration must not auto-expand scheduler catalog")
		}
	}
	if configuredImagesMainModel(nil) != defaultImagesMainModel {
		t.Fatal("nil manifest changed legacy model")
	}
}
