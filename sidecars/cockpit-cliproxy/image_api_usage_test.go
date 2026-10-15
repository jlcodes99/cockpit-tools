package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func imageAPITestUsageRecord(t *testing.T, s *relayServer, status int) usagePayload {
	t.Helper()
	if len(s.policy.tracker.records) != 1 {
		t.Fatalf("usage request count=%d", len(s.policy.tracker.records))
	}
	for id, records := range s.policy.tracker.records {
		if len(records) != 1 || records[0].Usage.TotalTokens != 604 {
			t.Fatalf("usage missing/duplicated: %#v", records)
		}
		payload, ok := s.policy.tracker.finalize(id, usageFinalizeInput{status: status})
		if !ok || payload.Usage.TotalTokens != 604 || payload.Success != (status < 400) {
			t.Fatalf("client outcome overwrote usage: %#v", payload)
		}
		return payload
	}
	panic("unreachable")
}

func TestImageAPIUsageSurvivesLocalFailuresAcrossEndpoints(t *testing.T) {
	for _, route := range []string{"generate", "edit", "responses", "responses-stream"} {
		for _, output := range []string{"url-404", "private-url", "bad-base64", "fake-image", "empty", "success"} {
			t.Run(route+"/"+output, func(t *testing.T) {
				download := httptest.NewServer(http.NotFoundHandler())
				defer download.Close()
				upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					image := map[string]any{"b64_json": imageAPITestPNGBase64}
					switch output {
					case "url-404":
						image = map[string]any{"url": "http://images.test/expired"}
					case "private-url":
						image = map[string]any{"url": "http://127.0.0.1/secret"}
					case "bad-base64":
						image = map[string]any{"b64_json": "bad!"}
					case "fake-image":
						image = map[string]any{"b64_json": "c2VjcmV0"}
					case "empty":
						image = map[string]any{}
					}
					_ = json.NewEncoder(w).Encode(map[string]any{"data": []any{image}, "usage": map[string]any{"input_tokens": 4, "output_tokens": 600, "total_tokens": 604}})
				}))
				defer upstream.Close()
				s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
				s.imageDownloads = imageAPITestDownloadPolicy(t, download.URL)
				path := imagesGenerationsPath
				body := `{"model":"gpt-image-2.5","prompt":"draw"}`
				if route == "edit" {
					path = imagesEditsPath
					body = `{"model":"gpt-image-2.5","prompt":"edit","image":"data:image/png;base64,` + imageAPITestPNGBase64 + `"}`
				} else if strings.HasPrefix(route, "responses") {
					path = "/v1/responses"
					body = `{"model":"chat-model","input":"draw","tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation"}],"stream":` + map[bool]string{true: "true", false: "false"}[route == "responses-stream"] + `}`
				}
				resp := imageAPITestRequest(s, path, body)
				status := 502
				if output == "success" {
					status = 200
				}
				if resp.Code != status {
					t.Fatalf("%d %s", resp.Code, resp.Body.String())
				}
				imageAPITestUsageRecord(t, s, status)
			})
		}
	}
}

func TestImageAPIUsageRetainedWhenDownloadCancelled(t *testing.T) {
	entered := make(chan struct{})
	download := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(entered)
		<-r.Context().Done()
	}))
	defer download.Close()
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{"data":[{"url":"http://images.test/image"}],"usage":{"input_tokens":4,"output_tokens":600,"total_tokens":604}}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	s.imageDownloads = imageAPITestDownloadPolicy(t, download.URL)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	req := httptest.NewRequest(http.MethodPost, imagesGenerationsPath, strings.NewReader(`{"model":"gpt-image-2.5","prompt":"draw"}`)).WithContext(ctx)
	req.Header.Set("Authorization", "Bearer client-key")
	req.Header.Set("Content-Type", "application/json")
	resp := httptest.NewRecorder()
	finished := make(chan struct{})
	go func() { s.router().ServeHTTP(resp, req); close(finished) }()
	select {
	case <-entered:
	case <-time.After(time.Second):
		t.Fatal("download not entered")
	}
	cancel()
	select {
	case <-finished:
	case <-time.After(time.Second):
		t.Fatal("download not cancelled")
	}
	if resp.Code != 499 {
		t.Fatalf("cancel status=%d %s", resp.Code, resp.Body.String())
	}
	imageAPITestUsageRecord(t, s, resp.Code)
}

func TestImageAPIFailedDeliveryChargesTokenLimitExactlyOnce(t *testing.T) {
	for _, valid := range []bool{false, true} {
		upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			b64 := "bad!"
			if valid {
				b64 = imageAPITestPNGBase64
			}
			_ = json.NewEncoder(w).Encode(map[string]any{"data": []any{map[string]any{"b64_json": b64}}, "usage": map[string]any{"input_tokens": 4, "output_tokens": 600, "total_tokens": 604}})
		}))
		s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
		key := &s.manifest.APIKeys[0]
		key.TokenLimit = 604
		s.policy.tokenLimiter = newAPIKeyTokenLimiter(s.manifest)
		s.policy.emitter = &eventEmitter{}
		resp := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
		want := 502
		if valid {
			want = 200
		}
		used, limit, exceeded := s.policy.tokenLimiter.exceeded(key)
		if resp.Code != want || used != 604 || limit != 604 || !exceeded || len(s.policy.tracker.records) != 0 {
			t.Fatalf("usage not charged once: status=%d used=%d limit=%d exceeded=%v", resp.Code, used, limit, exceeded)
		}
		upstream.Close()
	}
}

func TestImageAPIResponsesKeepsIndependentKeyModelsAndDisabledSource(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"`+imageAPITestPNGBase64+`"}]}`)
	}))
	defer upstream.Close()
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	key := &s.manifest.APIKeys[0]
	key.ProviderGateway = nil
	key.AllowedModels = []string{"chat-model", defaultImagesToolModel}
	key.ExcludedModels = []string{defaultImagesToolModel}
	body := `{"model":"chat-model","input":"draw","tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation"}]}`
	resp := imageAPITestRequest(s, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`)
	if resp.Code != 404 || calls.Load() != 0 {
		t.Fatal("direct Images API model filter changed")
	}
	resp = imageAPITestRequest(s, "/v1/responses", body)
	if resp.Code != 200 || calls.Load() != 1 {
		t.Fatalf("independent image source lost: %d calls=%d %s", resp.Code, calls.Load(), resp.Body.String())
	}
	s.manifest.accountByID["images-key"].ImageGenerationPolicy = "disabled"
	resp = imageAPITestRequest(s, "/v1/responses", body)
	if resp.Code != 404 || calls.Load() != 1 {
		t.Fatalf("disabled source was submitted: %d calls=%d", resp.Code, calls.Load())
	}
}
