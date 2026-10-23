package executor

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestCodexHTTPEncryptedRecoveryEffectiveIdentityHeaders(t *testing.T) {
	for _, mode := range []string{"execute", "stream", "compact"} {
		for _, changedHeader := range []string{"Authorization", "Chatgpt-Account-Id"} {
			t.Run(mode+"/"+changedHeader, func(t *testing.T) {
				requests := &codexRecoveryRequests{}
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					if requests.record(r) == 1 {
						w.WriteHeader(400)
						_, _ = io.WriteString(w, codexRecovery400)
						return
					}
					codexRecoveryTestSuccess(w, mode == "compact")
				}))
				defer server.Close()
				auth := codexRecoveryTestAuth(server.URL, t.Name())
				auth.Attributes["header:Authorization"] = "$X-Override-Auth"
				auth.Attributes["header:Chatgpt-Account-Id"] = "$X-Override-Account"
				cfg := &config.Config{}
				cfg.DisableImageGeneration = config.DisableImageGenerationAll
				e := NewCodexExecutor(cfg)
				payload := codexRecoveryTestPayload(t, t.Name())
				headers := http.Header{"X-Override-Auth": []string{"Bearer account-a"}, "X-Override-Account": []string{"account-a"}}
				call := func() {
					opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Headers: headers}
					req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: payload}
					if mode == "compact" {
						opts.Alt = "responses/compact"
					}
					if mode == "stream" {
						result, err := e.ExecuteStream(context.Background(), auth, req, opts)
						if err != nil {
							t.Fatal(err)
						}
						for chunk := range result.Chunks {
							if chunk.Err != nil {
								t.Fatal(chunk.Err)
							}
						}
					} else if _, err := e.Execute(context.Background(), auth, req, opts); err != nil {
						t.Fatal(err)
					}
				}
				call()
				call()
				if changedHeader == "Authorization" {
					headers.Set("X-Override-Auth", "Bearer account-b")
				} else {
					headers.Set("X-Override-Account", "account-b")
				}
				call()
				bodies, wireHeaders, _ := requests.snapshot()
				if len(bodies) != 4 {
					t.Fatalf("requests=%d want 4", len(bodies))
				}
				if gjson.GetBytes(bodies[2], "input.0.encrypted_content").Exists() {
					t.Fatal("same effective identity did not reuse rejection")
				}
				if wireHeaders[0].Get(changedHeader) == wireHeaders[3].Get(changedHeader) {
					t.Fatal("fixture did not change effective identity header")
				}
				if !gjson.GetBytes(bodies[3], "input.0.encrypted_content").Exists() {
					t.Fatal("rejection crossed effective identity header")
				}
			})
		}
	}
}
