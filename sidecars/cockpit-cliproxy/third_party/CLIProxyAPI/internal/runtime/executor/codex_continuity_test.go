package executor

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

type codexContinuityHook struct {
	mu         sync.Mutex
	binds      []cliproxyexecutor.ContinuityBindRequest
	events     []json.RawMessage
	finished   []cliproxyexecutor.ContinuityFinishRequest
	observeErr error
	bindErr    error
	releases   []cliproxyexecutor.ContinuityReleaseRequest
	entered    chan struct{}
	release    chan struct{}
	done       chan cliproxyexecutor.ContinuityFinishRequest
}

func (h *codexContinuityHook) Prepare(_ context.Context, r cliproxyexecutor.ContinuityPrepareRequest) (cliproxyexecutor.ContinuityPrepareResponse, error) {
	return cliproxyexecutor.ContinuityPrepareResponse{Ticket: uuid.NewString(), PreferredAccountID: "account", SelfContained: true, Body: r.Body, Headers: r.Headers}, nil
}
func (h *codexContinuityHook) Bind(_ context.Context, r cliproxyexecutor.ContinuityBindRequest) (cliproxyexecutor.ContinuityBindResponse, error) {
	h.mu.Lock()
	h.binds = append(h.binds, r)
	h.mu.Unlock()
	if h.bindErr != nil {
		return cliproxyexecutor.ContinuityBindResponse{}, h.bindErr
	}
	headers := r.Headers.Clone()
	if headers == nil {
		headers = http.Header{}
	}
	headers.Set("Session-Id", "bound-session")
	headers.Set("Thread-Id", "bound-thread")
	return cliproxyexecutor.ContinuityBindResponse{Ticket: r.Ticket, AttemptID: r.AttemptID, Body: r.Body, Headers: headers}, nil
}
func (h *codexContinuityHook) Observe(ctx context.Context, r cliproxyexecutor.ContinuityObserveRequest) (cliproxyexecutor.ContinuityObserveResponse, error) {
	terminal := false
	for _, event := range r.Events {
		if h.entered != nil {
			select {
			case h.entered <- struct{}{}:
			default:
			}
			select {
			case <-h.release:
			case <-ctx.Done():
				return cliproxyexecutor.ContinuityObserveResponse{}, ctx.Err()
			}
		}
		if h.observeErr != nil {
			return cliproxyexecutor.ContinuityObserveResponse{}, h.observeErr
		}
		h.mu.Lock()
		h.events = append(h.events, bytes.Clone(event))
		h.mu.Unlock()
		terminal = terminal || gjson.GetBytes(event, "type").String() == "response.completed" || gjson.GetBytes(event, "object").String() == "response.compaction"
	}
	return cliproxyexecutor.ContinuityObserveResponse{NextSequence: r.Sequence + 1, Terminal: terminal}, nil
}
func (h *codexContinuityHook) Finish(_ context.Context, r cliproxyexecutor.ContinuityFinishRequest) error {
	h.mu.Lock()
	h.finished = append(h.finished, r)
	h.mu.Unlock()
	if h.done != nil {
		h.done <- r
	}
	return nil
}

func (h *codexContinuityHook) Release(_ context.Context, r cliproxyexecutor.ContinuityReleaseRequest) error {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.releases = append(h.releases, r)
	return nil
}

func codexContinuityContext(t *testing.T, h *codexContinuityHook, body []byte, compact ...bool) context.Context {
	t.Helper()
	canonical := sdktranslator.TranslateRequest(sdktranslator.FormatOpenAIResponse, sdktranslator.FormatCodex, "gpt-5.4", bytes.Clone(body), true)
	canonical = cliproxyexecutor.NormalizeResponsesCallIDs(canonical, true)
	ctx, _, err := cliproxyexecutor.PrepareContinuity(cliproxyexecutor.WithContinuityHook(context.Background(), h), cliproxyexecutor.ContinuityPrepareRequest{Body: canonical, Compact: len(compact) > 0 && compact[0], Headers: http.Header{"Session-Id": []string{"logical-session"}}})
	if err != nil {
		t.Fatal(err)
	}
	return ctx
}

func codexContinuityConfig() *config.Config {
	cfg := &config.Config{SDKConfig: config.SDKConfig{DisableImageGeneration: config.DisableImageGenerationAll}}
	cfg.Codex.IdentityConfuse = true
	return cfg
}

func TestCodexContinuityHTTPPreservesOpaqueInputAndObservesRawTerminal(t *testing.T) {
	for _, mode := range []string{"nonstream", "stream", "compact"} {
		t.Run(mode, func(t *testing.T) {
			body := []byte(`{"model":"gpt-5.4","previous_response_id":"logical-response","prompt_cache_key":"logical-session","client_metadata":{"thread_id":"logical-thread"},"input":[{"type":"reasoning","id":"` + strings.Repeat("item-id-", 12) + `","encrypted_content":"synthetic-opaque-cipher","summary":[]}]}`)
			hook := &codexContinuityHook{done: make(chan cliproxyexecutor.ContinuityFinishRequest, 1)}
			wireItem := `{"type":"reasoning","id":"rs-wire","encrypted_content":"synthetic-output","summary":[],"signature":"synthetic-signature","unknown":{"keep":true}}`
			wireDone := `{"type":"response.output_item.done","output_index":0,"item":` + wireItem + `}`
			wireTerminal := `{"type":"response.completed","response":{"id":"resp-wire","status":"completed","output":[]}}`
			compactResponse := `{"id":"compact-id","object":"response.compaction","output":[{"type":"compaction","encrypted_content":"synthetic-compact"}]}`
			captured := make(chan []byte, 1)
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Header.Get("Session-Id") != "bound-session" || r.Header.Get("Thread-Id") != "bound-thread" {
					t.Errorf("continuity identity was rewritten: %v", r.Header)
				}
				actual, _ := io.ReadAll(r.Body)
				captured <- actual
				if mode == "compact" {
					w.Header().Set("Content-Type", "application/json")
					_, _ = io.WriteString(w, compactResponse)
					return
				}
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, "data: "+wireDone+"\n\ndata: "+wireTerminal+"\n\n")
			}))
			defer server.Close()
			auth := &cliproxyauth.Auth{ID: "account", Provider: "codex", Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic-token"}, Metadata: map[string]any{"access_token": "synthetic-token"}}
			ctx := codexContinuityContext(t, hook, body, mode == "compact")
			exec := NewCodexExecutor(codexContinuityConfig())
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Headers: http.Header{"Session-Id": []string{"logical-session"}}}
			req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: body}
			var publicPayloads [][]byte
			if mode == "stream" {
				opts.Stream = true
				result, err := exec.ExecuteStream(ctx, auth, req, opts)
				if err != nil {
					t.Fatal(err)
				}
				for chunk := range result.Chunks {
					if chunk.Err != nil {
						t.Fatal(chunk.Err)
					}
					publicPayloads = append(publicPayloads, bytes.Clone(chunk.Payload))
				}
			} else {
				if mode == "compact" {
					opts.Alt = "responses/compact"
				}
				result, err := exec.Execute(ctx, auth, req, opts)
				if err != nil {
					t.Fatal(err)
				}
				publicPayloads = append(publicPayloads, bytes.Clone(result.Payload))
			}
			actual := <-captured
			if gjson.GetBytes(actual, "input").Raw != gjson.GetBytes(body, "input").Raw || gjson.GetBytes(actual, "previous_response_id").String() != "logical-response" {
				t.Fatalf("opaque history changed: %s", actual)
			}
			select {
			case finish := <-hook.done:
				if finish.Status != "succeeded" {
					t.Fatalf("finish=%+v", finish)
				}
			case <-time.After(3 * time.Second):
				t.Fatal("missing finish")
			}
			hook.mu.Lock()
			defer hook.mu.Unlock()
			if len(hook.binds) != 1 {
				t.Fatalf("binds=%d", len(hook.binds))
			}
			if mode != "compact" {
				if len(hook.events) != 2 || !bytes.Equal(hook.events[0], []byte(wireDone)) {
					t.Fatal("observer changed the original complete done item")
				}
				assertCodexContinuityOutputItems(t, hook.events[1], "response.output", wireItem)
				public := codexContinuityPublicResponse(t, publicPayloads, mode == "stream")
				assertCodexContinuityOutputItems(t, public, "output", wireItem)
				if gjson.GetBytes(public, "id").String() != "resp-wire" {
					t.Fatal("public response identity changed")
				}
			} else if len(hook.events) != 1 || !bytes.Equal(hook.events[0], []byte(compactResponse)) || len(publicPayloads) != 1 || !bytes.Equal(publicPayloads[0], []byte(compactResponse)) {
				t.Fatal("compact raw observer or public response was changed")
			}
		})
	}
}

func assertCodexContinuityOutputItems(t *testing.T, payload []byte, path string, expected ...string) {
	t.Helper()
	items := gjson.GetBytes(payload, path).Array()
	if len(items) != len(expected) {
		t.Fatalf("output items=%d, want %d", len(items), len(expected))
	}
	for i, item := range items {
		if item.Raw != expected[i] {
			t.Fatalf("output item %d differs from the original upstream done item", i)
		}
	}
}

func codexContinuityPublicResponse(t *testing.T, payloads [][]byte, stream bool) []byte {
	t.Helper()
	if !stream {
		if len(payloads) != 1 || !json.Valid(payloads[0]) {
			t.Fatal("missing public JSON response")
		}
		return payloads[0]
	}
	for _, payload := range payloads {
		for _, line := range bytes.Split(payload, []byte("\n")) {
			line = bytes.TrimSpace(bytes.TrimPrefix(line, []byte("data:")))
			if gjson.GetBytes(line, "type").String() == "response.completed" {
				return []byte(gjson.GetBytes(line, "response").Raw)
			}
		}
	}
	t.Fatal("missing public completed event")
	return nil
}

func TestCodexContinuitySparseTerminalObservationPreservesHTTPWire(t *testing.T) {
	hook := &codexContinuityHook{done: make(chan cliproxyexecutor.ContinuityFinishRequest, 1)}
	ctx := codexContinuityContext(t, hook, []byte(`{"input":[]}`))
	attempt, _, _, err := cliproxyexecutor.BindContinuity(ctx, "account", nil, []byte(`{"input":[]}`))
	if err != nil {
		t.Fatal(err)
	}
	item := `{"id":"wire-item","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"synthetic answer"}],"unknown":{"keep":true}}`
	done := `{"type":"response.output_item.done","output_index":0,"item":` + item + `}`
	terminal := `{"type":"response.completed","response":{"id":"wire-response","status":"completed","output":[]}}`
	wire := "event: response.output_item.done\r\ndata: " + done + "\r\n\r\nevent: response.completed\r\ndata: " + terminal + "\r\n\r\n"
	response := &http.Response{StatusCode: http.StatusOK, Header: http.Header{"Content-Type": []string{"text/event-stream"}}, Body: io.NopCloser(strings.NewReader(wire))}
	if err := helps.ObserveContinuityHTTPResponse(ctx, attempt, response, nil); err != nil {
		t.Fatal(err)
	}
	actual, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatal(err)
	}
	if err := response.Body.Close(); err != nil {
		t.Fatal(err)
	}
	if string(actual) != wire {
		t.Fatal("durable observation changed raw SSE transport bytes")
	}
	if len(hook.events) != 2 || string(hook.events[0]) != done {
		t.Fatal("durable observer did not retain the original done event")
	}
	assertCodexContinuityOutputItems(t, hook.events[1], "response.output", item)
}

func TestCodexContinuityAcknowledgesBeforeClientOutputAndFailsClosed(t *testing.T) {
	for _, reject := range []bool{false, true} {
		t.Run(map[bool]string{false: "ack", true: "reject"}[reject], func(t *testing.T) {
			hook := &codexContinuityHook{entered: make(chan struct{}, 1), release: make(chan struct{}), done: make(chan cliproxyexecutor.ContinuityFinishRequest, 1)}
			if reject {
				hook.observeErr = errors.New("synthetic durable observation failure")
			}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp\",\"status\":\"completed\",\"output\":[]}}\n\n")
			}))
			defer server.Close()
			body := []byte(`{"input":[]}`)
			ctx := codexContinuityContext(t, hook, body)
			result, err := NewCodexExecutor(codexContinuityConfig()).ExecuteStream(ctx, &cliproxyauth.Auth{ID: "account", Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic"}}, cliproxyexecutor.Request{Model: "gpt-5.4", Payload: body}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: true})
			if err != nil {
				t.Fatal(err)
			}
			select {
			case <-hook.entered:
			case <-time.After(3 * time.Second):
				t.Fatal("observer not reached")
			}
			select {
			case chunk := <-result.Chunks:
				t.Fatalf("client received data before durable ACK: %+v", chunk)
			default:
			}
			close(hook.release)
			payloads, failures := 0, 0
			for chunk := range result.Chunks {
				if len(chunk.Payload) > 0 {
					payloads++
				}
				if chunk.Err != nil {
					failures++
				}
			}
			if reject && (payloads != 0 || failures != 1) {
				t.Fatalf("rejected observation payloads=%d failures=%d", payloads, failures)
			}
			if !reject && payloads == 0 {
				t.Fatal("acknowledged terminal was not forwarded")
			}
			<-hook.done
		})
	}
}

func TestCodexContinuityNeverStripsCipherOrRetriesRequestFault(t *testing.T) {
	var hits atomic.Int32
	body := []byte(`{"input":[{"type":"reasoning","id":"opaque-id","encrypted_content":"synthetic-opaque"}]}`)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		actual, err := io.ReadAll(r.Body)
		if err != nil || gjson.GetBytes(actual, "input").Raw != gjson.GetBytes(body, "input").Raw {
			t.Errorf("rejected upstream history changed: %s err=%v", actual, err)
		}
		w.WriteHeader(http.StatusBadRequest)
		_, _ = io.WriteString(w, `{"error":{"code":"invalid_encrypted_content","message":"encrypted content could not be verified"}}`)
	}))
	defer server.Close()
	hook := &codexContinuityHook{done: make(chan cliproxyexecutor.ContinuityFinishRequest, 1)}
	ctx := codexContinuityContext(t, hook, body)
	_, err := NewCodexExecutor(codexContinuityConfig()).Execute(ctx, &cliproxyauth.Auth{ID: "account", Provider: "codex", Attributes: map[string]string{"base_url": server.URL}, Metadata: map[string]any{"access_token": "synthetic-token"}}, cliproxyexecutor.Request{Model: "gpt-5.4", Payload: body}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse})
	if err == nil || hits.Load() != 1 {
		t.Fatalf("request fault replayed: hits=%d err=%v", hits.Load(), err)
	}
	if finish := <-hook.done; finish.Status != "failed" {
		t.Fatalf("finish=%+v", finish)
	}
}

func TestCodexContinuityWebsocketMultiTurnAndRawEvidence(t *testing.T) {
	for _, stream := range []bool{false, true} {
		t.Run(map[bool]string{false: "nonstream", true: "stream"}[stream], func(t *testing.T) {
			upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
			wireItem := `{"id":"rs-original-wire","type":"reasoning","encrypted_content":"synthetic-ws-output","summary":[],"signature":"synthetic-signature","unknown":["keep"]}`
			wireDone := `{"type":"response.output_item.done","output_index":0,"item":` + wireItem + `}`
			wireTerminal := `{"type":"response.completed","response":{"id":"resp-ws","status":"completed","output":[]}}`
			captured := make(chan []byte, 2)
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				conn, err := upgrader.Upgrade(w, r, nil)
				if err != nil {
					return
				}
				defer conn.Close()
				for range 2 {
					_, payload, err := conn.ReadMessage()
					if err != nil {
						return
					}
					captured <- payload
					_ = conn.WriteMessage(websocket.TextMessage, []byte(wireDone))
					_ = conn.WriteMessage(websocket.TextMessage, []byte(wireTerminal))
				}
			}))
			defer server.Close()
			hook := &codexContinuityHook{done: make(chan cliproxyexecutor.ContinuityFinishRequest, 2)}
			exec := NewCodexWebsocketsExecutor(codexContinuityConfig())
			session := uuid.NewString()
			defer exec.CloseExecutionSession(session)
			auth := &cliproxyauth.Auth{ID: "account", Provider: "codex", Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic-token"}}
			for range 2 {
				body := []byte(`{"previous_response_id":"logical-response","input":[{"type":"reasoning","id":"` + strings.Repeat("rs-wire-", 12) + `","encrypted_content":"synthetic-opaque","summary":[]}]}`)
				ctx := codexContinuityContext(t, hook, body)
				req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: body}
				opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: stream, Metadata: map[string]any{cliproxyexecutor.ExecutionSessionMetadataKey: session}}
				var publicPayloads [][]byte
				var observedWire [][]byte
				opts.WebSocketResponseObserver = func(_ context.Context, event cliproxyexecutor.WebSocketResponseEvent) {
					observedWire = append(observedWire, bytes.Clone(event.Payload))
				}
				if stream {
					result, err := exec.ExecuteStream(ctx, auth, req, opts)
					if err != nil {
						t.Fatal(err)
					}
					for chunk := range result.Chunks {
						if chunk.Err != nil {
							t.Fatal(chunk.Err)
						}
						publicPayloads = append(publicPayloads, bytes.Clone(chunk.Payload))
					}
				} else {
					result, err := exec.Execute(ctx, auth, req, opts)
					if err != nil {
						t.Fatal(err)
					}
					publicPayloads = append(publicPayloads, bytes.Clone(result.Payload))
				}
				actual := <-captured
				if gjson.GetBytes(actual, "input").Raw != gjson.GetBytes(body, "input").Raw || gjson.GetBytes(actual, "previous_response_id").String() != "logical-response" {
					t.Fatalf("WS modified bound opaque history: %s", actual)
				}
				select {
				case <-hook.done:
				case <-time.After(3 * time.Second):
					t.Fatal("WS finish not delivered")
				}
				if len(observedWire) != 2 || string(observedWire[0]) != wireDone || string(observedWire[1]) != wireTerminal {
					t.Fatal("continuity normalization changed raw WebSocket observer frames")
				}
				public := codexContinuityPublicResponse(t, publicPayloads, stream)
				assertCodexContinuityOutputItems(t, public, "output", wireItem)
				if gjson.GetBytes(public, "id").String() != "resp-ws" {
					t.Fatal("public WebSocket response identity changed")
				}
			}
			hook.mu.Lock()
			defer hook.mu.Unlock()
			if len(hook.binds) != 2 || hook.binds[0].AttemptID == hook.binds[1].AttemptID || hook.binds[0].Ticket == hook.binds[1].Ticket {
				t.Fatalf("WS turns reused ticket/attempt: %+v", hook.binds)
			}
			if len(hook.events) != 4 {
				t.Fatalf("WS durable events=%d, want four", len(hook.events))
			}
			for index, event := range hook.events {
				if index%2 == 0 {
					if string(event) != wireDone {
						t.Fatal("WS durable observer changed an original done event")
					}
				} else {
					assertCodexContinuityOutputItems(t, event, "response.output", wireItem)
				}
			}
		})
	}
}

func TestCodexContinuityWebsocketHTTPFallbackUsesDistinctAttempts(t *testing.T) {
	for _, stream := range []bool{false, true} {
		t.Run(map[bool]string{false: "nonstream", true: "stream"}[stream], func(t *testing.T) {
			var posts atomic.Int32
			body := []byte(`{"input":[{"role":"user","content":"continue old chat"},{"type":"reasoning","id":"old-id","encrypted_content":"unknown-old-cipher"}]}`)
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Method == http.MethodGet {
					w.WriteHeader(http.StatusUpgradeRequired)
					return
				}
				posts.Add(1)
				actual, _ := io.ReadAll(r.Body)
				if gjson.GetBytes(actual, "input").Raw != gjson.GetBytes(body, "input").Raw {
					t.Errorf("HTTP fallback changed old encrypted history: %s", actual)
				}
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-fallback\",\"status\":\"completed\",\"output\":[]}}\n\n")
			}))
			defer server.Close()
			hook := &codexContinuityHook{done: make(chan cliproxyexecutor.ContinuityFinishRequest, 3)}
			exec := NewCodexWebsocketsExecutor(codexContinuityConfig())
			ctx := codexContinuityContext(t, hook, body)
			auth := &cliproxyauth.Auth{ID: "account", Provider: "codex", Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic-token"}}
			req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: body}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: stream}
			var err error
			if stream {
				var result *cliproxyexecutor.StreamResult
				result, err = exec.ExecuteStream(ctx, auth, req, opts)
				if result != nil {
					for chunk := range result.Chunks {
						if chunk.Err != nil {
							err = chunk.Err
						}
					}
				}
			} else {
				_, err = exec.Execute(ctx, auth, req, opts)
			}
			if err != nil || posts.Load() != 1 {
				t.Fatalf("automatic fallback failed: posts=%d err=%v", posts.Load(), err)
			}
			hook.mu.Lock()
			defer hook.mu.Unlock()
			if len(hook.binds) != 2 || hook.binds[0].Ticket != hook.binds[1].Ticket || hook.binds[0].AttemptID == hook.binds[1].AttemptID {
				t.Fatalf("fallback reused attempt: %+v", hook.binds)
			}
		})
	}
}

func TestCodexContinuityManagerHTTPFailoverKeepsOneTicket(t *testing.T) {
	for _, stream := range []bool{false, true} {
		t.Run(map[bool]string{false: "nonstream", true: "stream"}[stream], func(t *testing.T) {
			var backupHits atomic.Int32
			body := []byte(`{"model":"gpt-5.4","input":[{"role":"user","content":"old chat"},{"type":"reasoning","id":"old-reasoning","encrypted_content":"unknown-old-cipher"},{"type":"function_call","call_id":"old-call","name":"send_message","namespace":"collaboration","arguments":"{\"message\":\"old-agent-cipher\"}"},{"type":"function_call_output","call_id":"old-call","output":"old result"},{"role":"user","content":"continue"}]}`)
			assertHistory := func(r *http.Request) {
				actual, _ := io.ReadAll(r.Body)
				if gjson.GetBytes(actual, "input").Raw != gjson.GetBytes(body, "input").Raw {
					t.Errorf("failover changed encrypted/tool history: %s", actual)
				}
			}
			original := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				assertHistory(r)
				w.WriteHeader(http.StatusTooManyRequests)
				_, _ = io.WriteString(w, `{"error":{"message":"synthetic quota exceeded"}}`)
			}))
			defer original.Close()
			backup := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				assertHistory(r)
				backupHits.Add(1)
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-backup\",\"status\":\"completed\",\"output\":[]}}\n\n")
			}))
			defer backup.Close()
			manager := cliproxyauth.NewManager(nil, nil, nil)
			manager.SetRetryConfig(3, 0, 3)
			manager.RegisterExecutor(NewCodexExecutor(codexContinuityConfig()))
			for id, endpoint := range map[string]string{"account": original.URL, "continuity-backup-account": backup.URL} {
				registry.GetGlobalRegistry().RegisterClient(id, "codex", []*registry.ModelInfo{{ID: "gpt-5.4"}})
				t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(id) })
				if _, err := manager.Register(context.Background(), &cliproxyauth.Auth{ID: id, Provider: "codex", Attributes: map[string]string{"base_url": endpoint, "api_key": "synthetic-token"}}); err != nil {
					t.Fatal(err)
				}
			}
			hook := &codexContinuityHook{done: make(chan cliproxyexecutor.ContinuityFinishRequest, 5)}
			ctx := cliproxyexecutor.WithContinuityHook(context.Background(), hook)
			req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: body}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: stream}
			var err error
			if stream {
				var result *cliproxyexecutor.StreamResult
				result, err = manager.ExecuteStream(ctx, []string{"codex"}, req, opts)
				if result != nil {
					for chunk := range result.Chunks {
						if chunk.Err != nil {
							err = chunk.Err
						}
					}
				}
			} else {
				_, err = manager.Execute(ctx, []string{"codex"}, req, opts)
			}
			if err != nil || backupHits.Load() != 1 {
				t.Fatalf("automatic failover failed: err=%v backup=%d", err, backupHits.Load())
			}
			hook.mu.Lock()
			defer hook.mu.Unlock()
			if len(hook.binds) != 2 || hook.binds[0].AccountID != "account" || hook.binds[1].AccountID != "continuity-backup-account" || hook.binds[0].Ticket != hook.binds[1].Ticket || hook.binds[0].AttemptID == hook.binds[1].AttemptID {
				t.Fatalf("failover continuity identities=%+v", hook.binds)
			}
			if len(hook.releases) != 1 {
				t.Fatalf("logical request releases=%d", len(hook.releases))
			}
		})
	}
}

func TestCodexContinuitySniffsWireFormatInsteadOfContentType(t *testing.T) {
	for _, mode := range []string{"nonstream", "stream", "compact"} {
		for _, contentType := range []string{"", "application/json", "text/event-stream"} {
			t.Run(mode+"/"+contentType, func(t *testing.T) {
				server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					if contentType != "" {
						w.Header().Set("Content-Type", contentType)
					} else {
						w.Header()["Content-Type"] = nil
					}
					if mode == "compact" {
						_, _ = io.WriteString(w, `{"id":"compact-sniffed","object":"response.compaction","output":[]}`)
					} else {
						_, _ = io.WriteString(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-sniffed\",\"status\":\"completed\",\"output\":[]}}\n\n")
					}
				}))
				defer server.Close()
				hook := &codexContinuityHook{done: make(chan cliproxyexecutor.ContinuityFinishRequest, 1)}
				body := []byte(`{"input":[]}`)
				ctx := codexContinuityContext(t, hook, body, mode == "compact")
				exec := NewCodexExecutor(codexContinuityConfig())
				auth := &cliproxyauth.Auth{ID: "account", Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic-token"}}
				req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: body}
				opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: mode == "stream"}
				if mode == "compact" {
					opts.Alt = "responses/compact"
				}
				if mode == "stream" {
					result, err := exec.ExecuteStream(ctx, auth, req, opts)
					if err != nil {
						t.Fatal(err)
					}
					for chunk := range result.Chunks {
						if chunk.Err != nil {
							t.Fatal(chunk.Err)
						}
					}
				} else {
					if _, err := exec.Execute(ctx, auth, req, opts); err != nil {
						t.Fatal(err)
					}
				}
				if finish := <-hook.done; finish.Status != "succeeded" {
					t.Fatalf("wire sniffing lost terminal: %+v", finish)
				}
			})
		}
	}
}

func TestCodexContinuityBindFailureReleasesPlanWithoutSending(t *testing.T) {
	for _, stream := range []bool{false, true} {
		t.Run(map[bool]string{false: "nonstream", true: "stream"}[stream], func(t *testing.T) {
			var hits atomic.Int32
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				hits.Add(1)
				w.WriteHeader(http.StatusInternalServerError)
			}))
			defer server.Close()
			manager := cliproxyauth.NewManager(nil, nil, nil)
			manager.SetRetryConfig(0, 0, 1)
			manager.RegisterExecutor(NewCodexExecutor(codexContinuityConfig()))
			registry.GetGlobalRegistry().RegisterClient("account", "codex", []*registry.ModelInfo{{ID: "continuity-bind-failure"}})
			t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient("account") })
			_, err := manager.Register(context.Background(), &cliproxyauth.Auth{ID: "account", Provider: "codex", Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic-token"}})
			if err != nil {
				t.Fatal(err)
			}
			hook := &codexContinuityHook{bindErr: errors.New("synthetic owner validation failure")}
			ctx := cliproxyexecutor.WithContinuityHook(context.Background(), hook)
			req := cliproxyexecutor.Request{Model: "continuity-bind-failure", Payload: []byte(`{"input":[]}`)}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: stream}
			if stream {
				_, err = manager.ExecuteStream(ctx, []string{"codex"}, req, opts)
			} else {
				_, err = manager.Execute(ctx, []string{"codex"}, req, opts)
			}
			if err == nil || hits.Load() != 0 {
				t.Fatalf("invalid binding was sent: err=%v hits=%d", err, hits.Load())
			}
			hook.mu.Lock()
			defer hook.mu.Unlock()
			if len(hook.releases) != 1 {
				t.Fatalf("failed Bind leaked request: releases=%d", len(hook.releases))
			}
		})
	}
}

func TestCodexContinuityMixedPreparationFailureCannotUseLegacyFallback(t *testing.T) {
	var hits atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer server.Close()
	want := errors.New("synthetic continuity prepare rejected")
	ctx := cliproxyexecutor.WithContinuityPreparationError(context.Background(), want)
	auth := &cliproxyauth.Auth{ID: "account", Provider: "codex", Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic-token"}}
	req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: []byte(`{"input":[{"type":"reasoning","encrypted_content":"synthetic-foreign"}]}`)}
	opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse}
	httpExec := NewCodexExecutor(codexContinuityConfig())
	wsExec := NewCodexWebsocketsExecutor(codexContinuityConfig())
	if _, err := httpExec.Execute(ctx, auth, req, opts); !errors.Is(err, want) {
		t.Fatalf("HTTP guard err=%v", err)
	}
	if _, err := httpExec.ExecuteStream(ctx, auth, req, opts); !errors.Is(err, want) {
		t.Fatalf("SSE guard err=%v", err)
	}
	if _, err := wsExec.Execute(ctx, auth, req, opts); !errors.Is(err, want) {
		t.Fatalf("WS guard err=%v", err)
	}
	if _, err := wsExec.ExecuteStream(ctx, auth, req, opts); !errors.Is(err, want) {
		t.Fatalf("WS stream guard err=%v", err)
	}
	opts.Alt = "responses/compact"
	if _, err := httpExec.executeCompact(ctx, auth, req, opts); !errors.Is(err, want) {
		t.Fatalf("compact guard err=%v", err)
	}
	if hits.Load() != 0 {
		t.Fatalf("failed prepare reached old transport: %d", hits.Load())
	}
}

func TestCodexContinuityCanonicalHistoryKeepsRequiredToolCallDefaults(t *testing.T) {
	for _, mode := range []string{"nonstream", "stream", "compact", "websocket"} {
		t.Run(mode, func(t *testing.T) {
			captured := make(chan []byte, 1)
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if mode == "websocket" {
					conn, err := (&websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}).Upgrade(w, r, nil)
					if err != nil {
						return
					}
					defer conn.Close()
					_, body, err := conn.ReadMessage()
					if err != nil {
						return
					}
					captured <- body
					_ = conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.completed","response":{"id":"resp-call-defaults","status":"completed","output":[]}}`))
					return
				}
				body, _ := io.ReadAll(r.Body)
				captured <- body
				if mode == "compact" {
					w.Header().Set("Content-Type", "application/json")
					_, _ = io.WriteString(w, `{"id":"compact-call-defaults","object":"response.compaction","output":[]}`)
				} else {
					w.Header().Set("Content-Type", "text/event-stream")
					_, _ = io.WriteString(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-call-defaults\",\"status\":\"completed\",\"output\":[]}}\n\n")
				}
			}))
			defer server.Close()
			body := []byte(`{"input":[{"type":"function_call","id":"fc_preserved","name":"lookup","arguments":"{}"},{"type":"function_call_output","id":"output_preserved","output":"ok"},{"type":"reasoning","id":"rs_preserved","encrypted_content":"synthetic-preserved","summary":[]}]}`)
			hook := &codexContinuityHook{done: make(chan cliproxyexecutor.ContinuityFinishRequest, 1)}
			ctx := codexContinuityContext(t, hook, body, mode == "compact")
			auth := &cliproxyauth.Auth{ID: "account", Provider: "codex", Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic-token"}}
			req := cliproxyexecutor.Request{Model: "gpt-5.4", Payload: body}
			opts := cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: mode == "stream"}
			if mode == "compact" {
				opts.Alt = "responses/compact"
			}
			exec := NewCodexExecutor(codexContinuityConfig())
			if mode == "websocket" {
				if _, err := NewCodexWebsocketsExecutor(codexContinuityConfig()).Execute(ctx, auth, req, opts); err != nil {
					t.Fatal(err)
				}
			} else if mode == "stream" {
				result, err := exec.ExecuteStream(ctx, auth, req, opts)
				if err != nil {
					t.Fatal(err)
				}
				for chunk := range result.Chunks {
					if chunk.Err != nil {
						t.Fatal(chunk.Err)
					}
				}
			} else {
				if _, err := exec.Execute(ctx, auth, req, opts); err != nil {
					t.Fatal(err)
				}
			}
			actual := <-captured
			if gjson.GetBytes(actual, "input.0.call_id").String() != "call_missing_0" || gjson.GetBytes(actual, "input.1.call_id").String() != "call_missing_0" {
				t.Fatalf("required tool call defaults were reverted: %s", actual)
			}
			if gjson.GetBytes(actual, "input.0.id").String() != "fc_preserved" || gjson.GetBytes(actual, "input.1.id").String() != "output_preserved" || gjson.GetBytes(actual, "input.2.id").String() != "rs_preserved" || gjson.GetBytes(actual, "input.2.encrypted_content").String() != "synthetic-preserved" {
				t.Fatalf("existing identities or ciphertext changed: %s", actual)
			}
			<-hook.done
		})
	}
}

func TestCodexContinuityCancelledHTTPKeepsFailureObservationWithoutLateCommit(t *testing.T) {
	for _, failure := range []bool{false, true} {
		t.Run(map[bool]string{false: "late-completion", true: "queued-quota-failure"}[failure], func(t *testing.T) {
			hook := &codexContinuityHook{done: make(chan cliproxyexecutor.ContinuityFinishRequest, 1)}
			ctx, cancel := context.WithCancel(codexContinuityContext(t, hook, []byte(`{"input":[]}`)))
			attempt, _, _, err := cliproxyexecutor.BindContinuity(ctx, "account", http.Header{}, []byte(`{"input":[]}`))
			if err != nil {
				t.Fatal(err)
			}
			event := `{"type":"response.completed","response":{"id":"late-success","status":"completed","output":[]}}`
			if failure {
				event = `{"type":"response.failed","response":{"status":"failed","error":{"code":"rate_limit_exceeded","message":"synthetic quota"}}}`
			}
			resp := &http.Response{StatusCode: http.StatusOK, Header: http.Header{}, Body: io.NopCloser(strings.NewReader("data: " + event + "\n\n"))}
			if err := helps.ObserveContinuityHTTPResponse(ctx, attempt, resp, nil); err != nil {
				t.Fatal(err)
			}
			cancel()
			data, readErr := io.ReadAll(resp.Body)
			if failure {
				if readErr != nil || !bytes.Contains(data, []byte("rate_limit_exceeded")) {
					t.Fatalf("queued authoritative failure lost: data=%s err=%v", data, readErr)
				}
			} else if !errors.Is(readErr, context.Canceled) || len(data) != 0 {
				t.Fatalf("late success escaped cancel fence: data=%s err=%v", data, readErr)
			}
			if err := resp.Body.Close(); err != nil {
				t.Fatal(err)
			}
			if finish := <-hook.done; finish.Status != "cancelled" {
				t.Fatalf("finish=%+v", finish)
			}
			hook.mu.Lock()
			defer hook.mu.Unlock()
			if len(hook.events) != 0 {
				t.Fatalf("cancelled attempt observed events: %s", hook.events)
			}
		})
	}
}
