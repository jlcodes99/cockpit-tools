package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestImageAPIModelFamilyBoundaries(t *testing.T) {
	for _, tc := range []struct{ model, family string }{
		{"gpt-image-2.5", defaultImagesToolModel}, {"gpt-image-2.5-flare", defaultImagesToolModel}, {"gpt-image-2.5-sunburst", defaultImagesToolModel},
		{"gpt-image-2", legacyImagesToolModel}, {"gpt-image-2-flare", legacyImagesToolModel},
		{"gpt-image-2.50", ""}, {"gpt-image-2.50-flare", ""}, {"gpt-image-20-flare", ""}, {"gpt-image-2.5-", ""}, {"gpt-image-2-", ""}, {"gpt-image-2.5-  ", ""},
	} {
		if got := imageAPIModelFamily(tc.model); got != tc.family {
			t.Errorf("%q family=%q want=%q", tc.model, got, tc.family)
		}
	}
}

func TestImageAPISelectedVariantMarksRuntimeActualModel(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{"gpt-image-2.5-flare"})
	s.manifest.accountByID["images-key"].ImageGenerationUpstream.Model = "gpt-image-2.5-flare"
	s.authManager = coreauth.NewManager(nil, nil, nil)
	_, err := s.authManager.Register(context.Background(), &coreauth.Auth{ID: "runtime-images", Provider: "codex", Attributes: map[string]string{"account_id": "images-key", "api_key": "upstream-secret"}})
	if err != nil {
		t.Fatal(err)
	}
	response := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if response.Code != http.StatusOK {
		t.Fatalf("%d %s", response.Code, response.Body.String())
	}
	auth, ok := s.authManager.GetByID("runtime-images")
	if !ok || auth.Success != 1 || auth.ModelStates["gpt-image-2.5-flare"] == nil || auth.ModelStates[defaultImagesToolModel] != nil {
		t.Fatalf("actual model was not reported to runtime: %#v", auth)
	}
	if s.policy.tracker.imageInFlightCount(auth.ID) != 0 || s.policy.tracker.accountInFlightCount(auth.ID) != 0 {
		t.Fatal("registered account slots were not released")
	}
}

func TestImageAPIExplicitVariantAcrossGenerationEditAndResponses(t *testing.T) {
	for _, tc := range []struct{ family, model string }{
		{defaultImagesToolModel, "gpt-image-2.5-flare"}, {defaultImagesToolModel, "gpt-image-2.5-sunburst"}, {legacyImagesToolModel, "gpt-image-2-flare"},
	} {
		t.Run(tc.model, func(t *testing.T) {
			for _, route := range []string{"generation", "edit", "responses", "responses-stream"} {
				t.Run(route, func(t *testing.T) {
					var calls atomic.Int32
					upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
						calls.Add(1)
						var model string
						if strings.HasPrefix(r.Header.Get("Content-Type"), "multipart/") {
							if err := r.ParseMultipartForm(1 << 20); err != nil {
								t.Error(err)
							}
							model = r.FormValue("model")
							if r.URL.Path != "/v1/images/edits" {
								t.Errorf("edit path %q", r.URL.Path)
							}
							file, _, err := r.FormFile("image[]")
							if err != nil {
								t.Error(err)
							} else {
								image, _ := io.ReadAll(file)
								_ = file.Close()
								if !bytes.Equal(image, imageAPITestPNG(t)) {
									t.Errorf("edit image %q", image)
								}
							}
						} else {
							var request map[string]any
							_ = json.NewDecoder(r.Body).Decode(&request)
							model, _ = request["model"].(string)
							if r.URL.Path != "/v1/images/generations" {
								t.Errorf("generation path %q", r.URL.Path)
							}
						}
						if model != tc.model {
							t.Errorf("suffix dropped: model %q want %q", model, tc.model)
						}
						_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}],"usage":{"input_tokens":3,"output_tokens":7,"total_tokens":10}}`)
					}))
					defer upstream.Close()
					s := imageAPITestRelay(t, upstream.URL+"/v1", []string{tc.model, tc.family, "gpt-image-2.5-another"})
					s.manifest.ImageGenerationModel = tc.family
					s.manifest.accountByID["images-key"].ImageGenerationUpstream.Model = tc.model
					path := imagesGenerationsPath
					body := `{"model":"` + tc.family + `","prompt":"draw"}`
					if route == "edit" {
						path = imagesEditsPath
						body = `{"model":"` + tc.family + `","prompt":"edit","image":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}`
					}
					if strings.HasPrefix(route, "responses") {
						path = "/v1/responses"
						body = `{"model":"chat-model","input":"draw","tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation","model":"gpt-image-2"}],"stream":` + map[bool]string{true: "true", false: "false"}[route == "responses-stream"] + `}`
					}
					response := imageAPITestRequest(s, path, body)
					if response.Code != http.StatusOK || calls.Load() != 1 {
						t.Fatalf("%d calls=%d %s", response.Code, calls.Load(), response.Body.String())
					}
					if strings.HasPrefix(route, "responses") && !strings.Contains(response.Body.String(), `"model":"`+tc.model+`"`) {
						t.Fatalf("Responses image item lost actual variant: %s", response.Body.String())
					}
					for _, records := range s.policy.tracker.records {
						for _, record := range records {
							if record.Model != tc.family || record.UpstreamModel != tc.model || record.Usage.TotalTokens != 10 {
								t.Fatalf("variant usage identity incorrect: %#v", record)
							}
						}
					}
				})
			}
		})
	}
}

func TestImageAPISelectedVariantRequiresFamilyAndExactCatalog(t *testing.T) {
	for _, tc := range []struct {
		name, family, selected string
		catalog                []string
		allowed                bool
	}{
		{"legacy bare", defaultImagesToolModel, "", []string{defaultImagesToolModel}, true},
		{"legacy suffix only", defaultImagesToolModel, "", []string{"gpt-image-2.5-flare"}, false},
		{"legacy empty catalog", defaultImagesToolModel, "", nil, false},
		{"missing selected variant", defaultImagesToolModel, "gpt-image-2.5-flare", []string{defaultImagesToolModel, "gpt-image-2.5-sunburst"}, false},
		{"wrong 2 family", legacyImagesToolModel, "gpt-image-2.5-flare", []string{"gpt-image-2.5-flare"}, false},
		{"wrong 2.5 family", defaultImagesToolModel, "gpt-image-2-flare", []string{"gpt-image-2-flare"}, false},
		{"2.50 not 2.5", defaultImagesToolModel, "gpt-image-2.50-flare", []string{"gpt-image-2.50-flare"}, false},
		{"empty suffix", defaultImagesToolModel, "gpt-image-2.5-", []string{"gpt-image-2.5-"}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var calls atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
			}))
			defer upstream.Close()
			s := imageAPITestRelay(t, upstream.URL, tc.catalog)
			s.manifest.ImageGenerationModel = tc.family
			s.manifest.accountByID["images-key"].ImageGenerationUpstream.Model = tc.selected
			response := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"`+tc.family+`","prompt":"draw"}`)
			if tc.allowed {
				if response.Code != http.StatusOK || calls.Load() != 1 {
					t.Fatalf("legacy manifest no longer works: %d %s", response.Code, response.Body.String())
				}
			} else if response.Code != http.StatusNotFound || calls.Load() != 0 {
				t.Fatalf("invalid variant caused generation: %d calls=%d %s", response.Code, calls.Load(), response.Body.String())
			}
		})
	}
}

func TestImageAPIExplicitSourceWithoutUpstreamCannotFallBackToConversation(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{"gpt-image-2.5-flare"})
	s.manifest.accountByID["images-key"].ImageGenerationUpstream = nil
	s.manifest.accountByID["images-key"].ImageGenerationPolicy = "enabled"
	response := imageAPITestRequest(s, "/v1/responses", `{"model":"chat-model","input":"draw","tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation","model":"gpt-image-2.5"}]}`)
	if response.Code != http.StatusNotFound || calls.Load() != 0 {
		t.Fatalf("invalid explicit image source fell back to conversation: %d calls=%d %s", response.Code, calls.Load(), response.Body.String())
	}
}

func TestImageAPISelectedVariantUsesActualExclusionAndBackoff(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{"gpt-image-2.5-flare", "gpt-image-2.5-sunburst"})
	source := s.manifest.accountByID["images-key"].ImageGenerationUpstream
	source.Model = "gpt-image-2.5-flare"
	s.manifest.AccountModelRules = []accountModelRule{{AccountID: "images-key", ExcludedModels: []string{source.Model}}}
	response := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if response.Code != http.StatusNotFound || calls.Load() != 0 {
		t.Fatalf("excluded exact variant reached provider: %d", response.Code)
	}
	source.Model = "gpt-image-2.5-sunburst"
	response = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if response.Code != http.StatusOK || calls.Load() != 1 {
		t.Fatalf("another variant was incorrectly excluded: %d %s", response.Code, response.Body.String())
	}
	s.manifest.AccountModelRules = []accountModelRule{{AccountID: "images-key", ExcludedModels: []string{defaultImagesToolModel}}}
	response = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if response.Code != http.StatusNotFound || calls.Load() != 1 {
		t.Fatalf("suffix bypassed the selected family's exclusion: %d calls=%d", response.Code, calls.Load())
	}
	s.manifest.AccountModelRules = nil
	s.providerBackoff.observe(providerGatewayBackoffKey{"images-key", upstream.URL, "gpt-image-2.5-flare"}, 429, "30", time.Now())
	source.Model = "gpt-image-2.5-flare"
	response = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if response.Code != http.StatusNotFound || calls.Load() != 1 {
		t.Fatalf("variant cooldown bypassed: %d calls=%d", response.Code, calls.Load())
	}
	source.Model = "gpt-image-2.5-sunburst"
	response = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if response.Code != http.StatusOK || calls.Load() != 2 {
		t.Fatalf("variant cooldown affected sibling: %d calls=%d", response.Code, calls.Load())
	}
}

func TestImageAPIMissingExplicitSourceCannotFallBackToConversation(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	delete(s.manifest.accountByID, "images-key")
	for _, tc := range []struct{ path, body string }{
		{imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`},
		{"/v1/responses", `{"model":"chat-model","input":"draw","tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation","model":"gpt-image-2.5"}]}`},
	} {
		response := imageAPITestRequest(s, tc.path, tc.body)
		if response.Code != http.StatusNotFound || calls.Load() != 0 {
			t.Fatalf("missing explicit image source fell back: %s %d calls=%d %s", tc.path, response.Code, calls.Load(), response.Body.String())
		}
	}
}

func TestImageAPIStaleFailureDoesNotDisableReplacedCredential(t *testing.T) {
	entered := make(chan struct{})
	complete := make(chan struct{})
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.Copy(io.Discard, r.Body)
		close(entered)
		<-complete
		w.WriteHeader(http.StatusUnauthorized)
		_, _ = io.WriteString(w, `{"error":{"message":"old credential rejected"}}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{"gpt-image-2.5-flare"})
	s.manifest.accountByID["images-key"].ImageGenerationUpstream.Model = "gpt-image-2.5-flare"
	s.authManager = coreauth.NewManager(nil, nil, nil)
	old, err := s.authManager.Register(context.Background(), &coreauth.Auth{ID: "runtime-images", Provider: "codex", Attributes: map[string]string{"account_id": "images-key", "api_key": "upstream-secret"}})
	if err != nil {
		t.Fatal(err)
	}
	finished := make(chan *httptest.ResponseRecorder, 1)
	go func() {
		finished <- imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	}()
	select {
	case <-entered:
	case <-time.After(time.Second):
		close(complete)
		t.Fatal("request did not reach upstream")
	}
	replacement, err := s.authManager.Register(context.Background(), &coreauth.Auth{ID: old.ID, Provider: "codex", Attributes: map[string]string{"account_id": "images-key", "api_key": "replacement-secret"}})
	if err != nil {
		close(complete)
		t.Fatal(err)
	}
	close(complete)
	var response *httptest.ResponseRecorder
	select {
	case response = <-finished:
	case <-time.After(time.Second):
		t.Fatal("request did not finish")
	}
	if response.Code != http.StatusUnauthorized {
		t.Fatalf("%d %s", response.Code, response.Body.String())
	}
	latest, ok := s.authManager.GetByID(old.ID)
	if !ok || latest.CredentialVersion != replacement.CredentialVersion || latest.RegistrationEpoch != replacement.RegistrationEpoch || latest.Failed != 0 || latest.LastError != nil || latest.ModelStates["gpt-image-2.5-flare"] != nil || latest.Unavailable {
		t.Fatalf("late failure damaged replaced credential: %#v", latest)
	}
}

func TestImageAPINativeAutomaticScopeHonorsExplicitDisabledPolicy(t *testing.T) {
	for _, disabled := range []bool{false, true} {
		name := "ready"
		if disabled {
			name = "explicit-disabled"
		}
		t.Run(name, func(t *testing.T) {
			var calls atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				var payload map[string]any
				_ = json.NewDecoder(r.Body).Decode(&payload)
				if payload["model"] != legacyImagesToolModel {
					t.Errorf("actual image model %#v", payload["model"])
				}
				_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
			}))
			defer upstream.Close()
			s := imageAPITestRelay(t, upstream.URL, []string{legacyImagesToolModel})
			s.manifest.ImageGenerationModel = legacyImagesToolModel
			key := &s.manifest.APIKeys[0]
			key.ProviderGateway = nil
			key.Internal = true
			key.AllowedModels = []string{"chat-model", legacyImagesToolModel}
			key.ModelRouting = &modelRoutingSpec{Automatic: true, NativeModels: []string{"chat-model", legacyImagesToolModel}}
			source := s.manifest.accountByID["images-key"]
			source.ImageGenerationUpstream.Model = legacyImagesToolModel
			source.ImageGenerationPolicy = "enabled"
			if disabled {
				source.ImageGenerationPolicy = "disabled"
			}
			response := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2","prompt":"draw"}`)
			if disabled {
				if response.Code != http.StatusNotFound || calls.Load() != 0 || !strings.Contains(response.Body.String(), "no selected image account supports model gpt-image-2") {
					t.Fatalf("explicit disabled policy bypassed or wrong rejection stage: %d calls=%d %s", response.Code, calls.Load(), response.Body.String())
				}
			} else if response.Code != http.StatusOK || calls.Load() != 1 {
				t.Fatalf("native automatic key rejected ready image source: %d calls=%d %s", response.Code, calls.Load(), response.Body.String())
			}
		})
	}
}

func TestImageAPIFamilyPoolKeepsEachAccountsSelectedVariant(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		index := calls.Add(1)
		var payload map[string]any
		_ = json.NewDecoder(r.Body).Decode(&payload)
		expectedModel, expectedKey := "gpt-image-2.5-flare", "Bearer upstream-secret"
		if index == 2 {
			expectedModel, expectedKey = "gpt-image-2.5-sunburst", "Bearer second-source-secret"
		}
		if r.URL.Path != imagesGenerationsPath || payload["model"] != expectedModel || r.Header.Get("Authorization") != expectedKey {
			t.Errorf("account's model/key mixed: call=%d path=%s model=%#v auth=%q", index, r.URL.Path, payload["model"], r.Header.Get("Authorization"))
		}
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}],"usage":{"input_tokens":4,"output_tokens":6,"total_tokens":10}}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{"gpt-image-2.5-flare", "gpt-image-2.5-sunburst"})
	first := s.manifest.accountByID["images-key"]
	first.ImageGenerationUpstream.Model = "gpt-image-2.5-flare"
	second := *first
	second.ID = "second-source"
	second.UpstreamAPIKey = "second-source-secret"
	second.ImageGenerationUpstream = &imageAPIUpstreamSpec{BaseURL: upstream.URL, APIKey: second.UpstreamAPIKey, Model: "gpt-image-2.5-sunburst", ModelIDs: []string{"gpt-image-2.5-flare", "gpt-image-2.5-sunburst"}}
	s.manifest.accountByID[second.ID] = &second
	s.manifest.APIKeys[0].ImageGenerationAccountIDs = []string{first.ID, second.ID}
	s.manifest.MaxConcurrentImageRequests = 1
	response := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2","prompt":"draw"}`)
	if response.Code != http.StatusOK {
		t.Fatalf("first source failed %d %s", response.Code, response.Body.String())
	}
	// Make the first source busy so the next request must use the second row's
	// independently selected variant rather than reusing the first row's model.
	if !s.policy.tracker.tryReserveImageJob("busy-first", first.ID, 1) {
		t.Fatal("could not reserve first source")
	}
	response = imageAPITestRequest(s, "/v1/responses", `{"model":"chat-model","input":"draw","stream":true,"tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation","model":"gpt-image-2"}]}`)
	if response.Code != http.StatusOK || !strings.Contains(response.Body.String(), `"model":"gpt-image-2.5-sunburst"`) {
		t.Fatalf("second row's actual model lost %d %s", response.Code, response.Body.String())
	}
	s.policy.tracker.releaseImageJobs("busy-first")
	seen := map[string]string{}
	for _, records := range s.policy.tracker.records {
		for _, record := range records {
			if record.Model != defaultImagesToolModel || record.Usage.TotalTokens != 10 {
				t.Fatalf("family usage changed %#v", record)
			}
			seen[record.AccountID] = record.UpstreamModel
		}
	}
	if calls.Load() != 2 || seen[first.ID] != "gpt-image-2.5-flare" || seen[second.ID] != "gpt-image-2.5-sunburst" {
		t.Fatalf("per-account actual model was not preserved: calls=%d usage=%#v", calls.Load(), seen)
	}
}
