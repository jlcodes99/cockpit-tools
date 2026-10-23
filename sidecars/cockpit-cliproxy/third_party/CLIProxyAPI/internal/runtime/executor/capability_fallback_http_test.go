package executor

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

type capabilityOrderSelector struct{}

func (capabilityOrderSelector) Pick(_ context.Context, _ string, _ string, _ cliproxyexecutor.Options, accounts []*cliproxyauth.Auth) (*cliproxyauth.Auth, error) {
	var selected *cliproxyauth.Auth
	for _, account := range accounts {
		if selected == nil || account.ID < selected.ID {
			selected = account
		}
	}
	return selected, nil
}

func TestManagerAutomaticCapabilityFallbackUsesExistingAuthorizedPoolBeforeOutput(t *testing.T) {
	plain := `{"input":[{"role":"user","content":"use tool"}],"tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]}`
	capabilityError := `{"error":{"type":"invalid_request_error","code":"unsupported_tool","message":"This endpoint does not support the requested tool"}}`
	for _, stream := range []bool{false, true} {
		for _, tc := range []struct {
			name, body, rejection         string
			pinned, partial, wantFallback bool
		}{
			{name: "capability", body: plain, rejection: capabilityError, wantFallback: true},
			{name: "bad parameter", body: plain, rejection: `{"error":{"type":"invalid_request_error","code":"invalid_value","param":"temperature","message":"Bad parameter"}}`},
			{name: "hard pin", body: plain, rejection: capabilityError, pinned: true},
			{name: "unresolved previous response", body: `{"previous_response_id":"private-response","input":"continue","tools":[{"type":"function","name":"lookup"}]}`, rejection: capabilityError},
			{name: "opaque history", body: `{"input":[{"type":"reasoning","encrypted_content":"private-signed-history","summary":[]}],"tools":[{"type":"function","name":"lookup"}]}`, rejection: capabilityError},
			{name: "unpaired tool result", body: `{"input":[{"type":"function_call_output","call_id":"missing","output":"result"}],"tools":[{"type":"function","name":"lookup"}]}`, rejection: capabilityError},
			{name: "partial output", body: plain, rejection: capabilityError, partial: true},
		} {
			mode := "http"
			if stream {
				mode = "stream"
			}
			t.Run(mode+"/"+tc.name, func(t *testing.T) {
				var first, backup atomic.Int32
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					_, _ = io.Copy(io.Discard, r.Body)
					if r.Header.Get("Authorization") == "Bearer first-key" {
						first.Add(1)
						if tc.partial {
							w.Header().Set("Content-Type", "text/event-stream")
							_, _ = io.WriteString(w, "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"started\"}\n\n")
							_, _ = io.WriteString(w, `data: {"type":"response.failed","response":{"id":"started","status":"failed","error":{"type":"invalid_request_error","code":"unsupported_tool","message":"execution error after output"}}}`+"\n\n")
							return
						}
						w.WriteHeader(http.StatusBadRequest)
						_, _ = io.WriteString(w, tc.rejection)
						return
					}
					backup.Add(1)
					w.Header().Set("Content-Type", "text/event-stream")
					_, _ = io.WriteString(w, `data: {"type":"response.completed","response":{"id":"backup-result","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"supported"}]}]}}`+"\n\n")
				}))
				defer server.Close()
				manager := cliproxyauth.NewManager(nil, capabilityOrderSelector{}, nil)
				manager.SetRetryConfig(0, 0, 0)
				cfg := &config.Config{}
				cfg.DisableImageGeneration = config.DisableImageGenerationAll
				manager.RegisterExecutor(NewCodexExecutor(cfg))
				model := "automatic-capability-fixture"
				firstID := t.Name() + "-a"
				for _, suffix := range []string{"a", "b"} {
					key := "first-key"
					if suffix == "b" {
						key = "backup-key"
					}
					account := &cliproxyauth.Auth{ID: t.Name() + "-" + suffix, Provider: "codex", Status: cliproxyauth.StatusActive, Attributes: map[string]string{"api_key": key, "base_url": server.URL}}
					registry.GetGlobalRegistry().RegisterClient(account.ID, "codex", []*registry.ModelInfo{{ID: model}})
					t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(account.ID) })
					if _, err := manager.Register(context.Background(), account); err != nil {
						t.Fatal(err)
					}
				}
				opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: stream}
				if tc.pinned {
					opts.Metadata = map[string]any{cliproxyexecutor.PinnedAuthMetadataKey: firstID}
				}
				req := cliproxyexecutor.Request{Model: model, Payload: []byte(tc.body)}
				var output []byte
				var err error
				if stream {
					var result *cliproxyexecutor.StreamResult
					result, err = manager.ExecuteStream(context.Background(), []string{"codex"}, req, opts)
					if err == nil {
						for chunk := range result.Chunks {
							output = append(output, chunk.Payload...)
							if chunk.Err != nil {
								err = chunk.Err
							}
						}
					}
				} else {
					var result cliproxyexecutor.Response
					result, err = manager.Execute(context.Background(), []string{"codex"}, req, opts)
					output = result.Payload
				}
				wantBackup := int32(0)
				if tc.wantFallback {
					wantBackup = 1
				}
				if first.Load() != 1 || backup.Load() != wantBackup {
					t.Fatalf("first=%d backup=%d wantBackup=%d err=%v", first.Load(), backup.Load(), wantBackup, err)
				}
				if tc.wantFallback && (err != nil || !strings.Contains(string(output), "supported")) {
					t.Fatalf("fallback result=%s err=%v", output, err)
				}
			})
		}
	}
}
