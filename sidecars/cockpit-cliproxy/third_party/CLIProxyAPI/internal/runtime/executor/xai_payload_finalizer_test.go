package executor

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestXAIPayloadFinalizerCompactAndMedia(t *testing.T) {
	for _, kind := range []string{"compact", "image", "video"} {
		t.Run(kind, func(t *testing.T) {
			bodies := make(chan []byte, 1)
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				body, _ := io.ReadAll(r.Body)
				bodies <- body
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write([]byte(`{"id":"response","output":[],"data":[]}`))
			}))
			defer server.Close()
			cfg := &config.Config{Payload: config.PayloadConfig{Override: []config.PayloadRule{{Models: []config.PayloadModelRule{{Name: "grok-4.6"}}, Params: map[string]any{"stop": []string{"configured"}, "model": "grok-configured"}}}}}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse}
			switch kind {
			case "compact":
				opts.Alt = "responses/compact"
			case "image":
				opts.SourceFormat = sdktranslator.Format(xaiImageHandlerType)
			case "video":
				opts.SourceFormat = sdktranslator.Format(xaiVideoHandlerType)
			}
			auth := &cliproxyauth.Auth{ID: "finalizer-" + kind, Provider: "xai", Attributes: map[string]string{"base_url": server.URL, "api_key": "test"}}
			_, err := NewXAIExecutor(cfg).Execute(t.Context(), auth, cliproxyexecutor.Request{Model: "grok-4.6", Payload: []byte(`{"model":"grok-4.6","input":[],"prompt":"test"}`)}, opts)
			if err != nil {
				t.Fatal(err)
			}
			body := <-bodies
			if gjson.GetBytes(body, "stop.0").String() != "configured" || gjson.GetBytes(body, "model").String() != "grok-configured" {
				t.Fatalf("%s replaced final payload rules: %s", kind, body)
			}
		})
	}
}

func TestXAIPayloadFinalizerWebsocketRetryAppliesOncePerAttempt(t *testing.T) {
	prepared, err := NewXAIExecutor(&config.Config{}).prepareResponsesRequest(context.Background(), cliproxyexecutor.Request{Model: "grok-4.6", Payload: []byte(`{"model":"grok-4.6","input":[]}`)}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse}, true)
	if err != nil {
		t.Fatal(err)
	}
	calls := 0
	finalize := func(body []byte) []byte { calls++; return prepared.finalizePayload(body) }
	for attempt := 1; attempt <= 2; attempt++ {
		body := buildXAIWebsocketRequestBody(prepared.body, finalize)
		if calls != attempt || gjson.GetBytes(body, "type").String() != "response.create" {
			t.Fatalf("attempt=%d calls=%d body=%s", attempt, calls, body)
		}
	}
}
