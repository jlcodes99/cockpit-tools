package main

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
)

func TestImageAPIConfiguredFullIDAcrossAllEndpoints(t *testing.T) {
	for _, model := range []string{"gpt-image-2.5-flare", "gpt-image-2.5-sunburst", "grok-imagine-image", "my-image-vendor/custom-v1"} {
		t.Run(model, func(t *testing.T) {
			for _, route := range []string{"generate", "edit", "responses", "responses-stream"} {
				t.Run(route, func(t *testing.T) {
					var calls atomic.Int32
					upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
						calls.Add(1)
						var actual string
						if strings.HasPrefix(r.Header.Get("Content-Type"), "multipart/") {
							if err := r.ParseMultipartForm(1 << 20); err != nil {
								t.Error(err)
							}
							actual = r.FormValue("model")
							if r.URL.Path != imagesEditsPath {
								t.Errorf("edit path %s", r.URL.Path)
							}
						} else {
							var payload map[string]any
							_ = json.NewDecoder(r.Body).Decode(&payload)
							actual, _ = payload["model"].(string)
							if r.URL.Path != imagesGenerationsPath {
								t.Errorf("generation path %s", r.URL.Path)
							}
						}
						if actual != model {
							t.Errorf("full selected model lost: got %q want %q", actual, model)
						}
						_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}],"usage":{"input_tokens":2,"output_tokens":8,"total_tokens":10}}`)
					}))
					defer upstream.Close()
					s := imageAPITestRelay(t, upstream.URL+"/v1", []string{model, "gpt-image-2.5-other"})
					s.manifest.ImageGenerationModel = model
					s.manifest.accountByID["images-key"].ImageGenerationUpstream.Model = model
					if model == "gpt-image-2.5-sunburst" {
						// Complete global IDs do not require a separate legacy per-account mapping.
						s.manifest.accountByID["images-key"].ImageGenerationUpstream.Model = ""
					}
					path := imagesGenerationsPath
					body := `{"model":"` + model + `","prompt":"draw"}`
					if route == "edit" {
						path = imagesEditsPath
						body = `{"model":"` + model + `","prompt":"edit","image":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}`
					}
					if strings.HasPrefix(route, "responses") {
						path = "/v1/responses"
						body = `{"model":"chat-model","input":"draw","tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation","model":"gpt-image-2"}],"stream":` + map[bool]string{true: "true", false: "false"}[route == "responses-stream"] + `}`
					}
					response := imageAPITestRequest(s, path, body)
					if response.Code != http.StatusOK || calls.Load() != 1 {
						t.Fatalf("%d calls=%d %s", response.Code, calls.Load(), response.Body.String())
					}
					if strings.HasPrefix(route, "responses") && !strings.Contains(response.Body.String(), `"model":"`+model+`"`) {
						t.Fatalf("Responses dropped complete model: %s", response.Body.String())
					}
					for _, records := range s.policy.tracker.records {
						for _, record := range records {
							if record.Model != model || record.UpstreamModel != model || record.Usage.TotalTokens != 10 {
								t.Fatalf("full model usage identity lost: %#v", record)
							}
						}
					}
				})
			}
		})
	}
}

func TestImageAPIConfiguredFullIDDoesNotChooseSibling(t *testing.T) {
	for _, tc := range []struct {
		name, configured, actual string
		catalog                  []string
		disabled                 bool
	}{
		{"same family wrong explicit ID", "gpt-image-2.5-flare", "gpt-image-2.5-sunburst", []string{"gpt-image-2.5-flare", "gpt-image-2.5-sunburst"}, false},
		{"missing exact catalog", "gpt-image-2.5-flare", "gpt-image-2.5-flare", []string{"gpt-image-2.5-sunburst"}, false},
		{"missing model cannot infer sibling", "gpt-image-2.5-flare", "", []string{"gpt-image-2.5-sunburst"}, false},
		{"custom wrong explicit ID", "grok-imagine-image", "grok-imagine-image-quality", []string{"grok-imagine-image", "grok-imagine-image-quality"}, false},
		{"custom disabled", "grok-imagine-image", "grok-imagine-image", []string{"grok-imagine-image"}, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var calls atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
			}))
			defer upstream.Close()
			s := imageAPITestRelay(t, upstream.URL, tc.catalog)
			s.manifest.ImageGenerationModel = tc.configured
			source := s.manifest.accountByID["images-key"]
			source.ImageGenerationUpstream.Model = tc.actual
			if tc.disabled {
				source.ImageGenerationPolicy = "disabled"
			}
			response := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"`+tc.configured+`","prompt":"draw"}`)
			if response.Code != http.StatusNotFound || calls.Load() != 0 {
				t.Fatalf("wrong complete model called: %d calls=%d %s", response.Code, calls.Load(), response.Body.String())
			}
		})
	}
}

func TestImageAPICustomModelNeverFallsBackToOAuth(t *testing.T) {
	for _, withOAuth := range []bool{false, true} {
		for _, responses := range []bool{false, true} {
			s := imageAPITestRelay(t, "http://127.0.0.1:1", []string{"grok-imagine-image"})
			s.manifest.ImageGenerationModel = "grok-imagine-image"
			key := &s.manifest.APIKeys[0]
			key.ProviderGateway.UpstreamModels = append(key.ProviderGateway.UpstreamModels, "grok-imagine-image")
			if withOAuth {
				source := s.manifest.accountByID["images-key"]
				source.ImageGenerationUpstream = nil
				source.AuthKind = "oauth"
				source.PlanType = "plus"
			} else {
				key.ImageGenerationAccountIDs = nil
			}
			path := imagesGenerationsPath
			body := `{"model":"grok-imagine-image","prompt":"draw"}`
			if responses {
				path = "/v1/responses"
				body = `{"model":"chat-model","input":"draw","tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation","model":"gpt-image-2"}]}`
			}
			response := imageAPITestRequest(s, path, body)
			runtime := s.runtime.(*fakeRuntime)
			if response.Code != http.StatusNotFound || runtime.executeCalls != 0 || runtime.streamCalls != 0 {
				t.Fatalf("custom image fell back to OAuth: oauth=%v responses=%v %d runtime=%#v %s", withOAuth, responses, response.Code, runtime, response.Body.String())
			}
		}
	}
}

func TestImageAPINativeFullModelAllowsLegacyRequestAndHonorsKeyFilters(t *testing.T) {
	for _, mode := range []string{"ready", "allowed-filter", "excluded-filter"} {
		t.Run(mode, func(t *testing.T) {
			var calls atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				var payload map[string]any
				_ = json.NewDecoder(r.Body).Decode(&payload)
				if payload["model"] != "grok-imagine-image" {
					t.Errorf("wrong model %#v", payload["model"])
				}
				_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
			}))
			defer upstream.Close()
			s := imageAPITestRelay(t, upstream.URL, []string{"grok-imagine-image"})
			s.manifest.ImageGenerationModel = "grok-imagine-image"
			s.manifest.accountByID["images-key"].ImageGenerationUpstream.Model = "grok-imagine-image"
			key := &s.manifest.APIKeys[0]
			key.ProviderGateway = nil
			key.AllowedModels = []string{"chat-model", "grok-imagine-image"}
			if mode == "allowed-filter" {
				key.AllowedModels = []string{"chat-model"}
			}
			if mode == "excluded-filter" {
				key.ExcludedModels = []string{"grok-imagine-image"}
			}
			response := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2","prompt":"draw"}`)
			if mode == "ready" {
				if response.Code != http.StatusOK || calls.Load() != 1 {
					t.Fatalf("legacy request rejected complete configuration: %d %s", response.Code, response.Body.String())
				}
			} else if response.Code != http.StatusNotFound || calls.Load() != 0 {
				t.Fatalf("key permission bypassed: %d calls=%d", response.Code, calls.Load())
			}
		})
	}
}

func TestImageAPIUnconfiguredCustomModelIsRejected(t *testing.T) {
	s := imageAPITestRelay(t, "http://127.0.0.1:1", []string{defaultImagesToolModel})
	s.manifest.APIKeys[0].ImageGenerationAccountIDs = nil
	response := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"grok-imagine-image","prompt":"draw"}`)
	if response.Code != http.StatusBadRequest || s.runtime.(*fakeRuntime).executeCalls != 0 || s.runtime.(*fakeRuntime).streamCalls != 0 {
		t.Fatalf("unconfigured model was accepted: %d %s", response.Code, response.Body.String())
	}
}
