package executor

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func apiServiceTestAuth(url string) *cliproxyauth.Auth {
	return &cliproxyauth.Auth{
		ID: "api-service-policy-test", Provider: "codex", Status: cliproxyauth.StatusActive,
		Attributes: map[string]string{"base_url": url},
		Metadata:   map[string]any{"access_token": "test"},
	}
}

func apiServiceTestExecutor(ws bool, cfg *config.Config) cliproxyauth.ProviderExecutor {
	if ws {
		return NewCodexWebsocketsExecutor(cfg)
	}
	return NewCodexExecutor(cfg)
}

// Match the upstream classification instead of wrapping every capacity failure
// in a request-scoped 503/server_error. The same contract applies to API keys.
func TestCodexAPIServiceCapacityBeforeAndAfterOutput(t *testing.T) {
	cases := []struct {
		name, event, code string
		status            int
		bootstrapStatus   int
	}{
		{"overload", codexOverloadEvent, "server_is_overloaded", 502, 503},
		{"slow-down", `{"type":"response.failed","response":{"error":{"code":"slow_down","message":"busy"}}}`, "slow_down", 502, 0},
		{"model-capacity", `{"type":"response.failed","response":{"error":{"type":"invalid_request_error","message":"Selected model is at capacity. Please try a different model."}}}`, "", 429, 429},
		{"quota", `{"type":"response.failed","response":{"error":{"type":"usage_limit_reached","resets_in_seconds":60,"message":"quota exhausted"}}}`, "", 429, 429},
	}
	for _, ws := range []bool{false, true} {
		for _, apiKey := range []bool{false, true} {
			for _, generated := range []bool{false, true} {
				for _, tc := range cases {
					t.Run(fmt.Sprintf("%s/ws=%v/key=%v/generated=%v", tc.name, ws, apiKey, generated), func(t *testing.T) {
						events := []string{codexCreatedEvent}
						if generated {
							events = append(events, codexOutputAddedEvent)
						}
						events = append(events, tc.event)
						server := codexSSEServer(events...)
						if ws {
							server.Close()
							server = codexWebsocketServer(t, events...)
						}
						defer server.Close()
						cfg := codexBufferingConfig(true)
						auth := apiServiceTestAuth(server.URL)
						if apiKey {
							auth.Attributes["api_key"] = "test"
						}
						req, opts := codexTestRequest()
						ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
						defer cancel()
						result, err := apiServiceTestExecutor(ws, cfg).ExecuteStream(ctx, auth, req, opts)
						wantStatus := tc.status
						if tc.bootstrapStatus > 0 && !generated {
							if result != nil || err == nil {
								t.Fatalf("bootstrap failure leaked a stream: %v", err)
							}
							wantStatus = tc.bootstrapStatus
						} else {
							if result == nil || err != nil {
								t.Fatalf("failure must be delivered in stream: %v", err)
							}
							var output string
							output, err = drainChunks(result)
							if !strings.Contains(output, "response.created") {
								t.Fatalf("handshake lost: %s", output)
							}
							if generated && !strings.Contains(output, "response.output_item.added") {
								t.Fatalf("generated output lost: %s", output)
							}
						}
						if err == nil || statusCodeFromTestError(t, err) != wantStatus {
							t.Fatalf("status must match upstream %d: %v", wantStatus, err)
						}
						if tc.code != "" && !strings.Contains(err.Error(), tc.code) {
							t.Fatalf("upstream error code lost: %v", err)
						}
						var transient interface{ IsTransientRequestScoped() bool }
						if errors.As(err, &transient) && transient.IsTransientRequestScoped() {
							t.Fatalf("unexpected local capacity override: %v", err)
						}
					})
				}
			}
		}
	}
}

func TestCodexAPIServiceNoReplayAfterGeneratedOutput(t *testing.T) {
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		calls.Add(1)
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = fmt.Fprintf(w, "data: %s\n\ndata: %s\n\ndata: %s\n\n", codexCreatedEvent, codexOutputAddedEvent, codexOverloadEvent)
	}))
	defer server.Close()
	req, opts := codexTestRequest()
	result, err := NewCodexExecutor(codexBufferingConfig(true)).ExecuteStream(context.Background(), codexTestAuth(server.URL), req, opts)
	if err != nil {
		t.Fatal(err)
	}
	body, err := drainChunks(result)
	if err == nil || calls.Load() != 1 || strings.Count(body, "response.output_item.added") != 1 {
		t.Fatalf("generated output must not be replayed: calls=%d err=%v body=%s", calls.Load(), err, body)
	}
}

func TestCodexAPIServiceLeavesDownstreamVersionAndAPIKeyIdentityUnchanged(t *testing.T) {
	for _, apiKey := range []bool{false, true} {
		for _, ws := range []bool{false, true} {
			cfg := &config.Config{}
			headers := http.Header{"User-Agent": {"codex_cli_rs/0.140.0"}, "Version": {"0.140.0"}, "Originator": {"codex_cli_rs"}}
			auth := apiServiceTestAuth("")
			if apiKey {
				auth.Attributes["api_key"] = "test"
			}
			var got http.Header
			if ws {
				got = applyCodexWebsocketHeaders(context.Background(), http.Header{}, auth, "test", cfg, headers)
			} else {
				req, _ := http.NewRequest(http.MethodPost, "http://localhost/responses", nil)
				applyCodexHeadersFromSources(req, auth, "test", true, cfg, headers)
				got = req.Header
			}
			if got.Get("Version") != "0.140.0" {
				t.Fatalf("version was rewritten: %v", got)
			}
			if apiKey && (got.Get("User-Agent") != headers.Get("User-Agent") || got.Get("Originator") != headers.Get("Originator")) {
				t.Fatal("API key identity must remain unchanged")
			}
		}
	}
}
