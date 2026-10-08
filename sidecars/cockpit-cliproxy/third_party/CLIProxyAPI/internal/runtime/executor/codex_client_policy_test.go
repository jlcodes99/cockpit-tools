package executor

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
)

// Persisted restriction metadata must not reactivate the retired client filter.
// Exercise both transports and both response modes through the real executors.
func TestCodexLegacyClientPolicyDoesNotRestrictClients(t *testing.T) {
	for _, ws := range []bool{false, true} {
		for _, stream := range []bool{false, true} {
			for _, client := range []struct{ userAgent, originator string }{
				{"curl/8.0", ""},
				{"claude-code/1.0", ""},
				{"codex-tui/0.153.3", "codex-tui"},
			} {
				for _, allowThirdParty := range []bool{false, true} {
					t.Run(fmt.Sprintf("ws=%v/stream=%v/client=%s/legacy-allow=%v", ws, stream, client.userAgent, allowThirdParty), func(t *testing.T) {
						server := codexSSEServer(codexCreatedEvent, codexCompletedEventBody)
						if ws {
							server.Close()
							server = codexWebsocketServer(t, codexCreatedEvent, codexCompletedEventBody)
						}
						defer server.Close()
						auth := apiServiceTestAuth(server.URL)
						auth.Metadata["codex_cli_only"] = true
						auth.Metadata["codex_cli_only_allow_app_server"] = allowThirdParty
						auth.Metadata["codex_cli_only_allow_app_server_clients"] = allowThirdParty
						req, opts := codexTestRequest()
						opts.Headers = http.Header{"User-Agent": {client.userAgent}, "Originator": {client.originator}}
						executor := apiServiceTestExecutor(ws, &config.Config{})
						ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
						defer cancel()
						var body string
						if stream {
							result, err := executor.ExecuteStream(ctx, auth, req, opts)
							if err != nil {
								t.Fatal(err)
							}
							body, err = drainChunks(result)
							if err != nil {
								t.Fatal(err)
							}
						} else {
							result, err := executor.Execute(ctx, auth, req, opts)
							if err != nil {
								t.Fatal(err)
							}
							body = string(result.Payload)
						}
						if !strings.Contains(body, "hello") {
							t.Fatalf("client blocked or response lost: %s", body)
						}
					})
				}
			}
		}
	}
}

func TestCodexCompactKeepsCredentialsWithoutLocalClientRestriction(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/responses/compact" || r.Header.Get("Authorization") != "Bearer test" {
			t.Errorf("compact route or credentials changed: path=%s", r.URL.Path)
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":"compact-ok","output":[]}`))
	}))
	defer server.Close()
	auth := apiServiceTestAuth(server.URL)
	auth.Metadata["codex_cli_only"] = true
	req, opts := codexTestRequest()
	opts.Alt = "responses/compact"
	opts.Headers = http.Header{"User-Agent": {"curl/8.0"}}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	result, err := NewCodexExecutor(&config.Config{}).Execute(ctx, auth, req, opts)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(result.Payload), "compact-ok") {
		t.Fatalf("unexpected compact response: %s", result.Payload)
	}
}
