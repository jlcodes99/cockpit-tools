package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

// Capture the actual Go relay's outbound request, not a second implementation
// of its mapping rules. This never starts an upstream server or uses credentials.
func TestParity22MediaImageReference(t *testing.T) {
	gin.SetMode(gin.TestMode)
	const png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="
	var cases []map[string]any
	for _, test := range []struct{ name, path, body string }{
		{"conversation-generation", "/v1/images/generations", `{"model":"gpt-5.5","prompt":"draw"}`},
		{"conversation-edit", "/v1/images/edits", `{"model":"gpt-5.5","prompt":"draw","image":"data:image/png;base64,` + png + `"}`},
		{"conversation-edit-multipart", "/v1/images/edits", `{"model":"gpt-5.5","prompt":"draw"}`},
		{"mixed-oauth-main", "/v1/responses", `{"model":"gpt-5.5","input":"draw","stream":true,"tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation","model":"gpt-image-2"}]}`},
	} {
		chunks := make(chan cliproxyexecutor.StreamChunk, 1)
		chunks <- cliproxyexecutor.StreamChunk{Payload: []byte("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"image_generation_call\",\"result\":\"AQ==\"}]}}\n\n")}
		close(chunks)
		runtime := &fakeRuntime{streamResult: &cliproxyexecutor.StreamResult{Chunks: chunks}}
		key := &apiKeySpec{ID: "fixture", Key: "fixture-key", Enabled: true,
			AccountIDs: []string{"chat"}, ImageGenerationAccountIDs: []string{"oauth", "api"},
			ProviderGateway: &providerGatewaySpec{BaseURL: "http://127.0.0.1:9/v1", APIKey: "fixture-provider-key", UpstreamModel: "vendor/private-model", UpstreamModels: []string{"vendor/private-model"}, WireAPI: "responses"}}
		oauth := &accountSpec{ID: "oauth", AuthKind: "oauth", PlanType: "plus", ImageGenerationPolicy: "enabled"}
		api := &accountSpec{ID: "api", AuthKind: "api_key", ImageGenerationPolicy: "enabled",
			ImageGenerationUpstream: &imageAPIUpstreamSpec{BaseURL: "http://127.0.0.1:9/v1", APIKey: "fixture-image-key", ModelIDs: []string{"gpt-image-2.5"}}}
		m := &manifest{ImageGenerationModel: "gpt-image-2.5", ImageGenerationMainModel: "gpt-5.5",
			ModelIDs:      []string{"vendor/private-model", "gpt-image-2.5"},
			aliasToSource: map[string]string{"gpt-5.5": "vendor/private-model"},
			apiKeyByValue: map[string]*apiKeySpec{"fixture-key": key},
			accountByID:   map[string]*accountSpec{"oauth": oauth, "api": api}}
		s := &relayServer{runtime: runtime, cfg: &config.Config{}, manifest: m, policy: &requestPolicy{manifest: m}}
		request := httptest.NewRequest(http.MethodPost, test.path, bytes.NewBufferString(test.body))
		request.Header.Set("Authorization", "Bearer fixture-key")
		request.Header.Set("Content-Type", "application/json")
		if strings.HasSuffix(test.name, "-multipart") {
			var body bytes.Buffer
			writer := multipart.NewWriter(&body)
			_ = writer.WriteField("model", "gpt-5.5")
			_ = writer.WriteField("prompt", "draw")
			part, err := writer.CreateFormFile("image", "fixture.png")
			if err != nil {
				t.Fatal(err)
			}
			image, err := base64.StdEncoding.DecodeString(png)
			if err != nil {
				t.Fatal(err)
			}
			if _, err = part.Write(image); err != nil {
				t.Fatal(err)
			}
			if err = writer.Close(); err != nil {
				t.Fatal(err)
			}
			request = httptest.NewRequest(http.MethodPost, test.path, &body)
			request.Header.Set("Authorization", "Bearer fixture-key")
			request.Header.Set("Content-Type", writer.FormDataContentType())
		}
		response := httptest.NewRecorder()
		s.router().ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Fatalf("%s: %d %s", test.name, response.Code, response.Body.String())
		}
		var wire map[string]any
		if err := json.Unmarshal(runtime.lastReq.Payload, &wire); err != nil {
			t.Fatal(err)
		}
		tool := wire["tools"].([]any)[0].(map[string]any)
		cases = append(cases, map[string]any{"name": test.name, "path": test.path,
			"request": json.RawMessage(test.body), "main_model": wire["model"], "image_model": tool["model"]})
	}
	parity22MediaFixture(t, "parity22_images_go.json", cases)
}

func parity22MediaFixture(t *testing.T, name string, value any) {
	t.Helper()
	encoded, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	encoded = append(encoded, '\n')
	dir, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	for {
		path := filepath.Join(dir, "src-tauri", "src", "modules", "codex_native_api", "fixtures", name)
		if _, err := os.Stat(filepath.Dir(path)); err == nil {
			if os.Getenv("COCKPIT_UPDATE_PARITY22_MEDIA") == "1" {
				if err := os.WriteFile(path, encoded, 0644); err != nil {
					t.Fatal(err)
				}
			} else {
				saved, err := os.ReadFile(path)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(saved, encoded) {
					t.Fatalf("Go media fixture drift: %s; regenerate with COCKPIT_UPDATE_PARITY22_MEDIA=1", path)
				}
			}
			return
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			t.Fatal("repository fixture directory not found")
		}
		dir = parent
	}
}
