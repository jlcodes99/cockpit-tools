package executor

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/translator"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	exec "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func managedCodexAuth(t *testing.T, base string) *coreauth.Auth {
	a := &coreauth.Auth{ID: "fixture", Provider: "codex", Attributes: map[string]string{"api_key": "fixture-token", "base_url": base, "header:X-Codex-Installation-Id": "spoof", "header:Cookie": "wrong-account-cookie"}}
	if err := helps.BindManagedIdentity(a, "account-a", &helps.ManagedIdentity{Version: 1, Scope: strings.Repeat("a", 64), InstallationID: "11111111-1111-4111-8111-111111111111", Revision: 1}); err != nil {
		t.Fatal(err)
	}
	return a
}
func TestCodexManagedIdentityHTTPStreamCompactAndGeneric(t *testing.T) {
	type seen struct {
		path    string
		headers http.Header
		body    []byte
	}
	var mu sync.Mutex
	var requests []seen
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		mu.Lock()
		requests = append(requests, seen{r.URL.Path, r.Header.Clone(), b})
		mu.Unlock()
		if strings.HasSuffix(r.URL.Path, "compact") || r.URL.Path == "/alpha/search" {
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"id":"r","object":"response","status":"completed","output":[]}`))
			return
		}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = w.Write([]byte("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[]}}\n\n"))
	}))
	defer server.Close()
	a := managedCodexAuth(t, server.URL)
	e := NewCodexExecutor(&config.Config{})
	raw := []byte(`{"model":"gpt-5.4","input":"hello","prompt_cache_key":"old-session","previous_response_id":"old-response","metadata":{"installation_id":"client"},"client_metadata":{"x-codex-installation-id":"client"}}`)
	options := exec.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Headers: http.Header{"User-Agent": {"compatible-client"}}}
	if _, err := e.Execute(context.Background(), a, exec.Request{Model: "gpt-5.4", Payload: raw}, options); err != nil {
		t.Fatal(err)
	}
	result, err := e.ExecuteStream(context.Background(), a, exec.Request{Model: "gpt-5.4", Payload: raw}, options)
	if err != nil {
		t.Fatal(err)
	}
	for chunk := range result.Chunks {
		if chunk.Err != nil {
			t.Fatal(chunk.Err)
		}
	}
	options.Alt = "responses/compact"
	if _, err := e.Execute(context.Background(), a, exec.Request{Model: "gpt-5.4", Payload: raw}, options); err != nil {
		t.Fatal(err)
	}
	req, _ := http.NewRequest("POST", server.URL+"/alpha/search", strings.NewReader(string(raw)))
	req.Header.Set("Content-Type", "application/json")
	response, err := e.HttpRequest(context.Background(), a, req)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = io.Copy(io.Discard, response.Body)
	_ = response.Body.Close()
	mu.Lock()
	defer mu.Unlock()
	if len(requests) != 4 {
		t.Fatal(len(requests))
	}
	for _, r := range requests {
		if r.headers.Get("Cookie") != "" {
			t.Fatal("caller cookie leaked", r.path)
		}
		if r.headers.Get("X-Codex-Installation-Id") != a.Attributes[helps.InstallationIDAttribute] {
			t.Fatal(r.path, r.headers)
		}
		if gjson.GetBytes(r.body, "client_metadata.x-codex-installation-id").String() != a.Attributes[helps.InstallationIDAttribute] {
			t.Fatal(r.path, string(r.body))
		}
		if gjson.GetBytes(r.body, "prompt_cache_key").String() != "old-session" {
			t.Fatal("legacy cache changed", r.path)
		}
	}
}
func TestCodexManagedIdentityWebsocketReusesOnlyMatchingGeneration(t *testing.T) {
	upgrader := websocket.Upgrader{}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		defer conn.Close()
		for {
			if _, _, err := conn.ReadMessage(); err != nil {
				return
			}
		}
	}))
	defer server.Close()
	a := managedCodexAuth(t, server.URL)
	a.Metadata = map[string]any{"task_id": "one", "agent_assertion": "one"}
	e := NewCodexWebsocketsExecutor(&config.Config{})
	s := &codexWebsocketSession{sessionID: "managed"}
	wsURL := "ws" + strings.TrimPrefix(server.URL, "http")
	first, closer, _, err := e.ensureUpstreamConn(context.Background(), a, s, a.ID, wsURL, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer closer.Close()
	a.Metadata["agent_assertion"] = "two"
	same, _, _, err := e.ensureUpstreamConn(context.Background(), a, s, a.ID, wsURL, nil)
	if err != nil || same != first {
		t.Fatal("assertion changed connection", err)
	}
	originalInstallation := a.Attributes[helps.InstallationIDAttribute]
	a.Metadata["task_id"] = "two"
	taskConn, taskCloser, _, taskErr := e.ensureUpstreamConn(context.Background(), a, s, a.ID, wsURL, nil)
	if taskErr != nil || taskConn == same || a.Attributes[helps.InstallationIDAttribute] != originalInstallation {
		t.Fatal("task generation failed", taskErr)
	}
	defer taskCloser.Close()
	same = taskConn
	for _, field := range []string{helps.IdentityRevisionAttribute, "api_key", helps.IdentityScopeAttribute} {
		a.Attributes[field] = map[string]string{helps.IdentityRevisionAttribute: "2", "api_key": "new-token", helps.IdentityScopeAttribute: strings.Repeat("b", 64)}[field]
		next, nextCloser, _, err := e.ensureUpstreamConn(context.Background(), a, s, a.ID, wsURL, nil)
		if err != nil || next == same {
			t.Fatal("stale connection", field, err)
		}
		defer nextCloser.Close()
		same = next
	}
}

func TestCodexManagedIdentityWebsocketActualRequests(t *testing.T) {
	for _, stream := range []bool{false, true} {
		t.Run(map[bool]string{false: "execute", true: "stream"}[stream], func(t *testing.T) {
			captured := make(chan []byte, 1)
			upgrader := websocket.Upgrader{}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Header.Get("Cookie") != "" {
					t.Error("WS caller cookie leaked")
				}
				if r.Header.Get("X-Codex-Installation-Id") != "11111111-1111-4111-8111-111111111111" {
					t.Error("managed handshake identity missing")
				}
				conn, err := upgrader.Upgrade(w, r, nil)
				if err != nil {
					t.Error(err)
					return
				}
				defer conn.Close()
				_, payload, err := conn.ReadMessage()
				if err != nil {
					t.Error(err)
					return
				}
				captured <- payload
				_ = conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.completed","response":{"id":"next","status":"completed","output":[]}}`))
			}))
			defer server.Close()
			a := managedCodexAuth(t, server.URL)
			e := NewCodexWebsocketsExecutor(&config.Config{})
			req := exec.Request{Model: "gpt-5.4", Payload: []byte(`{"model":"gpt-5.4","input":[{"type":"reasoning","encrypted_content":"` + validCodexReasoningEncryptedContentForTestSeed(76) + `"}],"previous_response_id":"old-response","prompt_cache_key":"session-old","client_metadata":{"installation_id":"client","session_id":"session-old"}}`)}
			opts := exec.Options{SourceFormat: sdktranslator.FromString("codex")}
			if stream {
				result, err := e.ExecuteStream(context.Background(), a, req, opts)
				if err != nil {
					t.Fatal(err)
				}
				for chunk := range result.Chunks {
					if chunk.Err != nil {
						t.Fatal(chunk.Err)
					}
				}
			} else {
				if _, err := e.Execute(context.Background(), a, req, opts); err != nil {
					t.Fatal(err)
				}
			}
			payload := <-captured
			for path, want := range map[string]string{"previous_response_id": "old-response", "prompt_cache_key": "session-old", "client_metadata.installation_id": a.Attributes[helps.InstallationIDAttribute], "input.0.encrypted_content": validCodexReasoningEncryptedContentForTestSeed(76)} {
				if gjson.GetBytes(payload, path).String() != want {
					t.Fatalf("%s: %s", path, payload)
				}
			}
		})
	}
}

func TestCodexManagedIdentityImagesActualRequests(t *testing.T) {
	for _, stream := range []bool{false, true} {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if r.Header.Get("Cookie") != "" {
				t.Error("image caller cookie leaked")
			}
			if r.URL.Path != "/images/generations" || r.Header.Get("X-Codex-Installation-Id") != "" {
				t.Error(r.URL.Path, r.Header)
			}
			body, _ := io.ReadAll(r.Body)
			if gjson.GetBytes(body, "metadata.installation_id").String() != "client" || gjson.GetBytes(body, "extra.installation_id").String() != "business" {
				t.Error(string(body))
			}
			if stream {
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, "event: image_generation.completed\ndata: {\"type\":\"image_generation.completed\",\"b64_json\":\"AA==\"}\n\n")
			} else {
				_, _ = io.WriteString(w, `{"created":1,"data":[{"b64_json":"AA=="}]}`)
			}
		}))
		a := managedCodexAuth(t, server.URL)
		e := NewCodexExecutor(&config.Config{})
		req := exec.Request{Model: "gpt-image-1.5", Payload: []byte(`{"model":"gpt-image-1.5","prompt":"draw","metadata":{"installation_id":"client"},"extra":{"installation_id":"business"}}`)}
		opts := codexOpenAIImageTestOptions(codexImagesGenerationsPath, stream)
		opts.Headers = http.Header{"x-codex-installation-id": {"caller-installation"}, "X-Codex-Installation-Id": {"caller-installation-two"}}
		if stream {
			result, err := e.ExecuteStream(context.Background(), a, req, opts)
			if err != nil {
				server.Close()
				t.Fatal(err)
			}
			for chunk := range result.Chunks {
				if chunk.Err != nil {
					t.Error(chunk.Err)
				}
			}
		} else {
			if _, err := e.Execute(context.Background(), a, req, opts); err != nil {
				t.Error(err)
			}
		}
		server.Close()
	}
}

func TestCodexManagedIdentityGenericAudioProtocolBoundary(t *testing.T) {
	for _, oauth := range []bool{false, true} {
		for _, path := range []string{"/v1/audio/speech", "/v1/audio/transcriptions", "/v1/audio/translations", "/backend-api/transcribe", "/alpha/search", "/v1/realtime"} {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				applies := oauth || r.URL.Path == "/alpha/search" || r.URL.Path == "/v1/realtime"
				header, field := "", "client"
				if applies {
					header = "11111111-1111-4111-8111-111111111111"
					field = header
				}
				body, _ := io.ReadAll(r.Body)
				if r.Header.Get("Cookie") != "" || gjson.GetBytes(body, "metadata.installation_id").String() != "business" {
					t.Error("business metadata changed or cookie leaked")
				}
				if r.Header.Get("X-Codex-Installation-Id") != header || gjson.GetBytes(body, "installation_id").String() != field {
					t.Error("protocol boundary", oauth, path, string(body), r.Header)
				}
				_, _ = io.WriteString(w, `{}`)
			}))
			a := managedCodexAuth(t, server.URL)
			if oauth {
				delete(a.Attributes, "api_key")
				a.Metadata = map[string]any{"access_token": "fixture"}
			}
			req, _ := http.NewRequest("POST", server.URL+path, strings.NewReader(`{"installation_id":"client","metadata":{"installation_id":"business"}}`))
			req.Header["x-codex-installation-id"] = []string{"caller-installation"}
			req.Header.Set("X-Codex-Installation-Id", "caller-installation-two")
			req.Header.Set("Content-Type", "application/json")
			resp, err := NewCodexExecutor(&config.Config{}).HttpRequest(context.Background(), a, req)
			if err != nil {
				t.Error(err)
			} else {
				_, _ = io.Copy(io.Discard, resp.Body)
				_ = resp.Body.Close()
			}
			server.Close()
		}
	}
}

func TestCodexManagedIdentityOAuthImageResponsesWinsModelHeaders(t *testing.T) {
	const model = "managed-identity-model"
	reg := registry.GetGlobalRegistry()
	reg.RegisterClient(model, "codex", []*registry.ModelInfo{{ID: model, Config: &registry.ModelConfig{OverrideHeader: map[string]string{"x-codex-installation-id": "model-spoof", "user-agent": "model-ua/1.0"}}}})
	t.Cleanup(func() { reg.UnregisterClient(model) })
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/responses" || r.Header.Get("X-Codex-Installation-Id") != "11111111-1111-4111-8111-111111111111" || r.Header.Get("User-Agent") != "model-ua/1.0" {
			t.Error("final header precedence", r.Header)
		}
		body, _ := io.ReadAll(r.Body)
		if !gjson.GetBytes(body, `tools.#(type=="image_generation")`).Exists() {
			t.Error("image tool missing", string(body))
		}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[]}}\n\n")
	}))
	defer server.Close()
	a := managedCodexAuth(t, server.URL)
	delete(a.Attributes, "api_key")
	a.Metadata = map[string]any{"access_token": "oauth-fixture"}
	_, err := NewCodexExecutor(&config.Config{}).Execute(context.Background(), a, exec.Request{Model: model, Payload: []byte(`{"model":"` + model + `","input":"draw","tools":[{"type":"image_generation"}]}`)}, exec.Options{SourceFormat: sdktranslator.FormatOpenAIResponse})
	if err != nil {
		t.Fatal(err)
	}
}
