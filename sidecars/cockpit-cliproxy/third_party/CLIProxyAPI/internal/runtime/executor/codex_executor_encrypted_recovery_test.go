package executor

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strconv"
	"sync"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/translator"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

const codexRecovery400 = `{"error":{"code":"invalid_encrypted_content","message":"Encrypted content could not be verified"}}`
const codexRecoveryCompleted = `data: {"type":"response.completed","response":{"id":"resp_recovered","status":"completed","model":"gpt-5.4","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}` + "\n\n"

type codexRecoveryRequests struct {
	sync.Mutex
	bodies  [][]byte
	headers []http.Header
	paths   []string
}

func (r *codexRecoveryRequests) record(req *http.Request) int {
	body, _ := io.ReadAll(req.Body)
	r.Lock()
	defer r.Unlock()
	r.bodies = append(r.bodies, body)
	r.headers = append(r.headers, req.Header.Clone())
	r.paths = append(r.paths, req.URL.Path)
	return len(r.bodies)
}
func (r *codexRecoveryRequests) snapshot() ([][]byte, []http.Header, []string) {
	r.Lock()
	defer r.Unlock()
	return append([][]byte(nil), r.bodies...), append([]http.Header(nil), r.headers...), append([]string(nil), r.paths...)
}
func codexRecoveryTestPayload(t *testing.T, session string) []byte {
	t.Helper()
	payload := map[string]any{"model": "gpt-5.4", "prompt_cache_key": session, "input": []any{
		map[string]any{"type": "reasoning", "id": "rs_stale", "encrypted_content": validCodexReasoningEncryptedContentForTestSeed(44), "summary": []any{map[string]any{"type": "summary_text", "text": "keep summary"}}},
		map[string]any{"type": "compaction", "encrypted_content": validCodexReasoningEncryptedContentForTestSeed(45)},
		map[string]any{"type": "message", "role": "user", "content": []any{map[string]any{"type": "input_text", "text": "keep user"}}},
		map[string]any{"type": "function_call", "call_id": "call_recovery", "name": "lookup", "arguments": "{}"},
		map[string]any{"type": "function_call_output", "call_id": "call_recovery", "output": "keep output"},
	}, "tools": []any{map[string]any{"type": "function", "name": "lookup", "parameters": map[string]any{"type": "object"}}}}
	body, err := json.Marshal(payload)
	if err != nil {
		t.Fatal(err)
	}
	return body
}
func codexRecoveryTestCall(ctx context.Context, e *CodexExecutor, auth *cliproxyauth.Auth, payload []byte, mode string, compat bool) error {
	req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: payload}
	if compat {
		req.Metadata = map[string]any{"cliproxy.resolved_api_key_model_info": &registry.ModelInfo{ID: "gpt-5.4", IsCompat: true}}
	}
	opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse}
	if mode == "compact" {
		opts.Alt = "responses/compact"
	}
	if mode != "stream" {
		_, err := e.Execute(ctx, auth, req, opts)
		return err
	}
	stream, err := e.ExecuteStream(ctx, auth, req, opts)
	if err != nil {
		return err
	}
	for chunk := range stream.Chunks {
		if chunk.Err != nil {
			return chunk.Err
		}
	}
	return nil
}
func codexRecoveryTestSuccess(w http.ResponseWriter, compact bool) {
	if compact {
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"id":"resp_compact","output":[{"type":"compaction","encrypted_content":"compact-result"}],"usage":{"input_tokens":1,"output_tokens":1}}`)
		return
	}
	w.Header().Set("Content-Type", "text/event-stream")
	_, _ = io.WriteString(w, codexRecoveryCompleted)
}
func codexRecoveryTestAuth(serverURL, id string) *cliproxyauth.Auth {
	return &cliproxyauth.Auth{ID: id, Provider: "codex", Attributes: map[string]string{"base_url": serverURL}, Metadata: map[string]any{"access_token": "oauth-test-access", "type": "codex", "refresh_owner": "cockpit_token_authority"}}
}
func assertCodexRecoveryPreservesContent(t *testing.T, body []byte) {
	t.Helper()
	if gjson.GetBytes(body, `input.#(type=="reasoning").encrypted_content`).Exists() || gjson.GetBytes(body, `input.#(type=="reasoning").id`).Exists() || gjson.GetBytes(body, `input.#(type=="compaction")`).Exists() {
		t.Fatalf("rejected content retained: %s", body)
	}
	if gjson.GetBytes(body, `input.#(type=="reasoning").summary.0.text`).String() != "keep summary" || gjson.GetBytes(body, `input.#(type=="message").content.0.text`).String() != "keep user" || gjson.GetBytes(body, `input.#(type=="function_call_output").output`).String() != "keep output" || gjson.GetBytes(body, "tools.0.name").String() != "lookup" {
		t.Fatalf("content changed during recovery: %s", body)
	}
}
func TestCodexHTTPEncryptedRecoveryExecuteStreamAndCompact(t *testing.T) {
	for _, mode := range []string{"execute", "stream", "compact"} {
		t.Run(mode, func(t *testing.T) {
			requests := &codexRecoveryRequests{}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				attempt := requests.record(r)
				if attempt == 1 {
					w.WriteHeader(http.StatusBadRequest)
					_, _ = io.WriteString(w, codexRecovery400)
					return
				}
				codexRecoveryTestSuccess(w, mode == "compact")
			}))
			defer server.Close()
			cfg := &config.Config{}
			cfg.DisableImageGeneration = config.DisableImageGenerationAll
			e := NewCodexExecutor(cfg)
			auth := codexRecoveryTestAuth(server.URL, t.Name())
			payload := codexRecoveryTestPayload(t, t.Name())
			if err := codexRecoveryTestCall(context.Background(), e, auth, payload, mode, false); err != nil {
				t.Fatal(err)
			}
			bodies, headers, paths := requests.snapshot()
			if len(bodies) != 2 {
				t.Fatalf("requests=%d want 2", len(bodies))
			}
			if !gjson.GetBytes(bodies[0], "input.0.encrypted_content").Exists() {
				t.Fatalf("first attempt unexpectedly stripped: %s", bodies[0])
			}
			assertCodexRecoveryPreservesContent(t, bodies[1])
			if headers[0].Get("Authorization") != "Bearer oauth-test-access" || headers[1].Get("Authorization") != headers[0].Get("Authorization") || headers[0].Get("Session-Id") != headers[1].Get("Session-Id") {
				t.Fatal("retry changed credentials or session")
			}
			if paths[0] != paths[1] {
				t.Fatal("retry changed endpoint")
			}
			if err := codexRecoveryTestCall(context.Background(), e, auth, payload, mode, false); err != nil {
				t.Fatal(err)
			}
			bodies, _, _ = requests.snapshot()
			if len(bodies) != 3 {
				t.Fatalf("lineage followup requests=%d", len(bodies))
			}
			assertCodexRecoveryPreservesContent(t, bodies[2])
			// Same client session on another account cannot inherit rejection history.
			other := codexRecoveryTestAuth(server.URL, t.Name()+"-other")
			if err := codexRecoveryTestCall(context.Background(), e, other, payload, mode, false); err != nil {
				t.Fatal(err)
			}
			bodies, _, _ = requests.snapshot()
			if !gjson.GetBytes(bodies[3], "input.0.encrypted_content").Exists() {
				t.Fatal("lineage crossed account")
			}
			different := codexRecoveryTestPayload(t, t.Name()+"-different-session")
			if err := codexRecoveryTestCall(context.Background(), e, auth, different, mode, false); err != nil {
				t.Fatal(err)
			}
			bodies, _, _ = requests.snapshot()
			if !gjson.GetBytes(bodies[4], "input.0.encrypted_content").Exists() {
				t.Fatal("lineage crossed session")
			}
			for _, replacement := range []string{"version", "epoch", "token", "account-id"} {
				renewed := codexRecoveryTestAuth(server.URL, auth.ID)
				switch replacement {
				case "version":
					renewed.CredentialVersion = 1
				case "epoch":
					renewed.RegistrationEpoch = 1
				case "token":
					renewed.Metadata["access_token"] = "reauthorized-token"
				case "account-id":
					renewed.Metadata["account_id"] = "reauthorized-account"
				}
				if err := codexRecoveryTestCall(context.Background(), e, renewed, payload, mode, false); err != nil {
					t.Fatal(err)
				}
				bodies, _, _ = requests.snapshot()
				if !gjson.GetBytes(bodies[len(bodies)-1], "input.0.encrypted_content").Exists() {
					t.Fatalf("lineage crossed new credential identity: %s", replacement)
				}
			}
		})
	}
}
func TestCodexHTTPEncryptedRecoveryOnlyExplicitErrors(t *testing.T) {
	tests := []struct {
		name, body                  string
		status                      int
		apiKey, compat, noEncrypted bool
		wantAttempts                int
	}{
		{name: "targeted signature", body: `{"error":{"code":"thinking_signature_invalid","message":"The encrypted content could not be verified. Reason: Encrypted content could not be decrypted or parsed."}}`, status: 400, wantAttempts: 2},
		{name: "ordinary signature", body: `{"error":{"code":"thinking_signature_invalid","message":"Invalid signature"}}`, status: 400, wantAttempts: 1},
		{name: "ordinary400", body: `{"error":{"code":"invalid_request_error","message":"bad request"}}`, status: 400, wantAttempts: 1},
		{name: "500", body: codexRecovery400, status: 500, wantAttempts: 1},
		{name: "apikey strict", body: codexRecovery400, status: 400, apiKey: true, wantAttempts: 1},
		{name: "apikey compat", body: codexRecovery400, status: 400, apiKey: true, compat: true, wantAttempts: 1},
		{name: "oauth compat", body: codexRecovery400, status: 400, compat: true, wantAttempts: 1},
		{name: "no encrypted", body: codexRecovery400, status: 400, noEncrypted: true, wantAttempts: 1},
	}
	for _, mode := range []string{"execute", "stream", "compact"} {
		for _, tc := range tests {
			t.Run(mode+"/"+tc.name, func(t *testing.T) {
				requests := &codexRecoveryRequests{}
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					if requests.record(r) == 1 {
						w.WriteHeader(tc.status)
						_, _ = io.WriteString(w, tc.body)
						return
					}
					codexRecoveryTestSuccess(w, mode == "compact")
				}))
				defer server.Close()
				auth := codexRecoveryTestAuth(server.URL, t.Name())
				if tc.apiKey {
					auth.Attributes["api_key"] = "strict-api-key"
				}
				payload := codexRecoveryTestPayload(t, t.Name())
				if tc.noEncrypted {
					payload = []byte(`{"model":"gpt-5.4","input":[{"role":"user","content":"hello"}]}`)
				}
				cfg := &config.Config{}
				cfg.DisableImageGeneration = config.DisableImageGenerationAll
				e := NewCodexExecutor(cfg)
				err := codexRecoveryTestCall(context.Background(), e, auth, payload, mode, tc.compat)
				bodies, _, _ := requests.snapshot()
				if len(bodies) != tc.wantAttempts {
					t.Fatalf("attempts=%d want %d err=%v", len(bodies), tc.wantAttempts, err)
				}
				if tc.wantAttempts == 2 && err != nil {
					t.Fatal(err)
				}
				if tc.wantAttempts == 1 && err == nil {
					t.Fatal("unexpected success")
				}
			})
		}
	}
}
func TestCodexHTTPEncryptedRecoverySecondRejectionAndCancel(t *testing.T) {
	for _, mode := range []string{"execute", "stream", "compact"} {
		for _, cancelBeforeRetry := range []bool{false, true} {
			t.Run(mode+"/cancel="+strconv.FormatBool(cancelBeforeRetry), func(t *testing.T) {
				ctx, cancel := context.WithCancel(context.Background())
				defer cancel()
				requests := &codexRecoveryRequests{}
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					requests.record(r)
					if cancelBeforeRetry {
						cancel()
					}
					w.WriteHeader(400)
					_, _ = io.WriteString(w, codexRecovery400)
				}))
				defer server.Close()
				cfg := &config.Config{}
				cfg.DisableImageGeneration = config.DisableImageGenerationAll
				e := NewCodexExecutor(cfg)
				err := codexRecoveryTestCall(ctx, e, codexRecoveryTestAuth(server.URL, t.Name()), codexRecoveryTestPayload(t, t.Name()), mode, false)
				bodies, _, _ := requests.snapshot()
				if err == nil {
					t.Fatal("unexpected success")
				}
				want := 2
				if cancelBeforeRetry {
					want = 1
					if !errors.Is(err, context.Canceled) {
						t.Fatalf("error=%v want context canceled", err)
					}
				}
				if len(bodies) != want {
					t.Fatalf("attempts=%d want %d", len(bodies), want)
				}
			})
		}
	}
}

func TestCodexHTTPEncryptedRecoveryNeverRetriesStreamFailureEvents(t *testing.T) {
	for _, mode := range []string{"execute", "stream"} {
		t.Run(mode, func(t *testing.T) {
			requests := &codexRecoveryRequests{}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				requests.record(r)
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, `data: {"type":"response.output_text.delta","delta":"already emitted"}`+"\n\n"+`data: {"type":"response.failed","response":{"error":{"code":"invalid_encrypted_content","type":"invalid_request_error","message":"Encrypted content could not be verified"}}}`+"\n\n")
			}))
			defer server.Close()
			cfg := &config.Config{}
			cfg.DisableImageGeneration = config.DisableImageGenerationAll
			err := codexRecoveryTestCall(context.Background(), NewCodexExecutor(cfg), codexRecoveryTestAuth(server.URL, t.Name()), codexRecoveryTestPayload(t, t.Name()), mode, false)
			if err == nil {
				t.Fatal("unexpected success")
			}
			bodies, _, _ := requests.snapshot()
			if len(bodies) != 1 {
				t.Fatalf("stream failure replayed after output: attempts=%d", len(bodies))
			}
		})
	}
}
