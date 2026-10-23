package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func imageAPITestRelay(t *testing.T, base string, models []string) *relayServer {
	t.Helper()
	gin.SetMode(gin.TestMode)
	account := accountSpec{ID: "images-key", AuthKind: "api_key", UpstreamAPIKey: "upstream-secret", ImageGenerationUpstream: &imageAPIUpstreamSpec{BaseURL: base, APIKey: "upstream-secret", ModelIDs: models}}
	m := &manifest{ImageGenerationModel: defaultImagesToolModel, ModelIDs: []string{"chat-model"}, Accounts: []accountSpec{account}, APIKeys: []apiKeySpec{{ID: "client", Key: "client-key", Enabled: true, AccountIDs: []string{"conversation-key"}, ImageGenerationAccountIDs: []string{"images-key"}, ProviderGateway: &providerGatewaySpec{BaseURL: base, APIKey: "conversation-secret", UpstreamModels: []string{"chat-model"}, UpstreamModel: "chat-model", WireAPI: "responses"}}}}
	// The gateway account is separate from the account executing the image API.
	m.apiKeyByValue = map[string]*apiKeySpec{"client-key": &m.APIKeys[0]}
	m.accountByID = map[string]*accountSpec{"images-key": &m.Accounts[0]}
	m.accountByAPIKey = map[string]*accountSpec{"upstream-secret": &m.Accounts[0]}
	tracker := newRequestUsageTracker()
	return &relayServer{manifest: m, cfg: &config.Config{}, policy: &requestPolicy{manifest: m, tracker: tracker, tokenLimiter: newAPIKeyTokenLimiter(m)}, runtime: &fakeRuntime{}}
}

func imageAPITestRequest(s *relayServer, path string, body string) *httptest.ResponseRecorder {
	req := httptest.NewRequest(http.MethodPost, path, strings.NewReader(body))
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.router().ServeHTTP(w, req)
	return w
}

func TestImageAPIModelScopeAndConfiguredModel(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.Path != "/v1/images/generations" {
			t.Errorf("path %q", r.URL.Path)
		}
		if r.Header.Get("Authorization") != "Bearer upstream-secret" {
			t.Error("wrong credential")
		}
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		if body["model"] != "gpt-image-2" || body["n"] != float64(1) || body["response_format"] != "b64_json" {
			t.Errorf("unexpected body %#v", body)
		}
		_, _ = io.WriteString(w, `{"created":100,"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}],"usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL+"/v1", []string{"gpt-image-2"})
	w := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if w.Code != http.StatusNotFound || calls.Load() != 0 {
		t.Fatalf("mismatch selected incompatible account: %d %s calls=%d", w.Code, w.Body.String(), calls.Load())
	}
	s.manifest.ImageGenerationModel = "gpt-image-2"
	w = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2","prompt":"draw","response_format":"url"}`)
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==") {
		t.Fatalf("%d %s", w.Code, w.Body.String())
	}
	if calls.Load() != 1 {
		t.Fatalf("calls=%d", calls.Load())
	}
	var record usagePayload
	for _, records := range s.policy.tracker.records {
		if len(records) > 0 {
			record = records[0]
		}
	}
	if record.AccountID != "images-key" || record.Model != "gpt-image-2" || record.Usage.InputTokens != 7 || record.Usage.OutputTokens != 3 || record.Usage.TotalTokens != 10 || !record.Usage.TokenBreakdown.Valid() {
		t.Fatalf("image usage was lost or billed to the conversation model: %#v", record)
	}
}

func TestImageAPIResponsesSSEAndOptionalTools(t *testing.T) {
	var images, chat atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/v1/images/generations" {
			images.Add(1)
			var body map[string]any
			_ = json.NewDecoder(r.Body).Decode(&body)
			if body["prompt"] != "latest prompt" {
				t.Errorf("prompt uses history: %#v", body)
			}
			_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==","revised_prompt":"improved"}],"usage":{"input_tokens":10,"output_tokens":20,"total_tokens":30}}`)
		} else {
			chat.Add(1)
			_, _ = io.WriteString(w, `{"id":"chat_response","output":[]}`)
		}
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	optional := `{"model":"chat-model","input":"hello","tools":[{"type":"image_generation","model":"gpt-image-2"}]}`
	w := imageAPITestRequest(s, "/v1/responses", optional)
	if w.Code != http.StatusOK || images.Load() != 0 || chat.Load() != 1 {
		t.Fatalf("optional tool triggered images: %d %s", w.Code, w.Body.String())
	}
	forced := `{"model":"chat-model","stream":true,"tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation","model":"gpt-image-2"}],"input":[{"role":"user","content":"previous"},{"role":"assistant","content":"old answer"},{"role":"user","content":[{"type":"input_text","text":"latest prompt"}]}]}`
	w = imageAPITestRequest(s, "/v1/responses", forced)
	if w.Code != http.StatusOK || images.Load() != 1 || chat.Load() != 1 {
		t.Fatalf("%d %s", w.Code, w.Body.String())
	}
	var added, done, completed map[string]any
	sequence := 0
	for _, payload := range imageFramePayloads(w.Body.Bytes()) {
		var event map[string]any
		if err := json.Unmarshal(payload, &event); err != nil {
			t.Fatal(err)
		}
		if event["sequence_number"] != float64(sequence) {
			t.Fatalf("invalid sequence %#v", event)
		}
		sequence++
		switch event["type"] {
		case "response.output_item.added":
			added = event
		case "response.output_item.done":
			done = event
		case "response.completed":
			completed = event
		}
	}
	if added == nil || done == nil || completed == nil {
		t.Fatalf("missing events: %s", w.Body.String())
	}
	if added["item"].(map[string]any)["id"] != done["item"].(map[string]any)["id"] {
		t.Fatal("tool item ID changed")
	}
	result := completed["response"].(map[string]any)
	if result["output"].([]any)[0].(map[string]any)["result"] != "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==" {
		t.Fatalf("missing image %#v", result)
	}
}

func TestImageAPIEditMultipartAndMask(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/images/edits" {
			t.Errorf("edit routed to %s", r.URL.Path)
		}
		if err := r.ParseMultipartForm(1 << 20); err != nil {
			t.Error(err)
		}
		if r.FormValue("model") != defaultImagesToolModel || r.FormValue("prompt") != "edit" || r.FormValue("response_format") != "b64_json" {
			t.Errorf("unexpected fields %#v", r.Form)
		}
		for _, name := range []string{"image[]", "mask"} {
			file, _, err := r.FormFile(name)
			if err != nil {
				t.Error(err)
				continue
			}
			data, _ := io.ReadAll(file)
			_ = file.Close()
			if !bytes.Equal(data, imageAPITestPNG(t)) {
				t.Errorf("unexpected %s bytes %q", name, data)
			}
		}
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL+"/v1", []string{defaultImagesToolModel})
	w := imageAPITestRequest(s, imagesEditsPath, `{"model":"gpt-image-2.5","prompt":"edit","image":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==","mask":{"image_url":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}}`)
	if w.Code != http.StatusOK {
		t.Fatalf("JSON edit %d %s", w.Code, w.Body.String())
	}
	var buf bytes.Buffer
	writer := multipart.NewWriter(&buf)
	_ = writer.WriteField("model", defaultImagesToolModel)
	_ = writer.WriteField("prompt", "edit")
	for _, name := range []string{"image[]", "mask"} {
		file, _ := writer.CreateFormFile(name, "image.png")
		_, _ = file.Write(imageAPITestPNG(t))
	}
	_ = writer.Close()
	req := httptest.NewRequest(http.MethodPost, imagesEditsPath, &buf)
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", writer.FormDataContentType())
	w = httptest.NewRecorder()
	s.router().ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("multipart edit %d %s", w.Code, w.Body.String())
	}
}

func TestImageAPIURLResultDoesNotSendCredential(t *testing.T) {
	var base string
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/download" {
			if r.Header.Get("Authorization") != "" {
				t.Error("credential leaked to image download")
			}
			_, _ = w.Write(imageAPITestPNG(t))
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"data": []any{map[string]any{"url": base + "/download"}}})
	}))
	defer upstream.Close()
	base = "http://images.test"
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	s.imageDownloads = imageAPITestDownloadPolicy(t, upstream.URL)
	w := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==") {
		t.Fatalf("%d %s", w.Code, w.Body.String())
	}
}

func TestImageAPIRejectsErrorsAndAsync(t *testing.T) {
	for _, tc := range []struct {
		name   string
		status int
		body   string
	}{{"auth", 401, `{"error":{"message":"bad key"}}`}, {"pending", 202, `{"id":"img_task","status":"pending"}`}, {"empty", 200, `{"data":[]}`}, {"bad-base64", 200, `{"data":[{"b64_json":"invalid!"}]}`}, {"bad-json", 200, `{`}} {
		t.Run(tc.name, func(t *testing.T) {
			var calls atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				w.WriteHeader(tc.status)
				_, _ = io.WriteString(w, tc.body)
			}))
			defer upstream.Close()
			s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
			w := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
			want := 502
			if tc.status == 401 {
				want = 401
			}
			if w.Code != want {
				t.Fatalf("%d %s", w.Code, w.Body.String())
			}
			w = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw","async":true}`)
			if w.Code != 400 || calls.Load() != 1 {
				t.Fatalf("async was submitted %d calls=%d", w.Code, calls.Load())
			}
		})
	}
}

func TestImageAPIRequestCancellationAndTimeout(t *testing.T) {
	for _, timeout := range []bool{false, true} {
		t.Run(map[bool]string{false: "cancel", true: "timeout"}[timeout], func(t *testing.T) {
			entered := make(chan struct{})
			cancelled := make(chan struct{})
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				_, _ = io.Copy(io.Discard, r.Body)
				close(entered)
				<-r.Context().Done()
				close(cancelled)
			}))
			defer upstream.Close()
			s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
			previous := imageAPIRequestTimeout
			if timeout {
				imageAPIRequestTimeout = 30 * time.Millisecond
			}
			defer func() { imageAPIRequestTimeout = previous }()
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			req := httptest.NewRequest(http.MethodPost, imagesGenerationsPath, strings.NewReader(`{"model":"gpt-image-2.5","prompt":"draw"}`)).WithContext(ctx)
			req.Header.Set("Authorization", "Bearer client-key")
			req.Header.Set("Content-Type", "application/json")
			w := httptest.NewRecorder()
			finished := make(chan struct{})
			go func() { s.router().ServeHTTP(w, req); close(finished) }()
			select {
			case <-entered:
			case <-time.After(time.Second):
				t.Fatal("upstream not entered")
			}
			if !timeout {
				cancel()
			}
			select {
			case <-finished:
			case <-time.After(time.Second):
				t.Fatal("request did not terminate")
			}
			select {
			case <-cancelled:
			case <-time.After(time.Second):
				t.Fatal("upstream not cancelled")
			}
			if timeout && w.Code != http.StatusGatewayTimeout {
				t.Fatalf("%d %s", w.Code, w.Body.String())
			}
			if s.policy.tracker.imageInFlightCount("images-key") != 0 {
				t.Fatal("image concurrency reservation leaked")
			}
		})
	}
}

func TestImageAPISourceSkipsPolicyAndCooldown(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	original := s.manifest.accountByID["images-key"]
	blocked := *original
	blocked.ID = "blocked"
	blocked.ImageGenerationPolicy = "disabled"
	s.manifest.accountByID[blocked.ID] = &blocked
	s.manifest.APIKeys[0].ImageGenerationAccountIDs = []string{"blocked", "images-key"}
	w := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if w.Code != 200 {
		t.Fatalf("blocked first account prevented healthy fallback: %d %s", w.Code, w.Body.String())
	}
	blocked.ImageGenerationPolicy = "enabled"
	s.providerBackoff.observe(providerGatewayBackoffKey{"blocked", upstream.URL, defaultImagesToolModel}, 429, "30", time.Now())
	w = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if w.Code != 200 {
		t.Fatalf("cooling first account prevented healthy fallback: %d %s", w.Code, w.Body.String())
	}
}

func TestImageAPIAccountExclusionAndConcurrency(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	s.manifest.AccountModelRules = []accountModelRule{{AccountID: "images-key", ExcludedModels: []string{defaultImagesToolModel}}}
	w := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if w.Code == http.StatusOK || calls.Load() != 0 {
		t.Fatalf("excluded account reached upstream: %d calls=%d", w.Code, calls.Load())
	}
	s.manifest.AccountModelRules = nil
	s.manifest.MaxConcurrentImageRequests = 1
	s.policy.tracker.tryReserveImageJob("existing-request", "images-key", 1)
	w = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if w.Code != http.StatusTooManyRequests || calls.Load() != 0 {
		t.Fatalf("image concurrency guard bypassed: %d calls=%d", w.Code, calls.Load())
	}
	s.policy.tracker.releaseImageJobs("existing-request")
	w = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw","stream":true}`)
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "event: image_generation.completed") || calls.Load() != 1 {
		t.Fatalf("slot was not recoverable or stream incomplete: %d %s", w.Code, w.Body.String())
	}
	s.manifest.MaxAccountConcurrency = 1
	s.policy.tracker.tryReserveAccountSlot("existing-request", "images-key", 1)
	w = imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if w.Code != http.StatusTooManyRequests || calls.Load() != 1 {
		t.Fatalf("account concurrency guard bypassed: %d calls=%d", w.Code, calls.Load())
	}
}

func TestImageAPIPoolPrefersSourceWithFreeSlots(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
	}))
	defer upstream.Close()
	for _, imageFull := range []bool{true, false} {
		s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
		alternative := *s.manifest.accountByID["images-key"]
		alternative.ID = "alternative"
		s.manifest.accountByID[alternative.ID] = &alternative
		s.manifest.APIKeys[0].ImageGenerationAccountIDs = []string{"images-key", "alternative"}
		if imageFull {
			s.manifest.MaxConcurrentImageRequests = 1
			s.policy.tracker.tryReserveImageJob("busy", "images-key", 1)
		} else {
			s.manifest.MaxAccountConcurrency = 1
			s.policy.tracker.tryReserveAccountSlot("busy", "images-key", 1)
		}
		w := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
		if w.Code != http.StatusOK {
			t.Fatalf("full=%v %d %s", imageFull, w.Code, w.Body.String())
		}
		for _, records := range s.policy.tracker.records {
			for _, record := range records {
				if record.AccountID != "alternative" {
					t.Fatalf("selected full account: %#v", record)
				}
			}
		}
	}
	if calls.Load() != 2 {
		t.Fatalf("calls=%d", calls.Load())
	}
}
