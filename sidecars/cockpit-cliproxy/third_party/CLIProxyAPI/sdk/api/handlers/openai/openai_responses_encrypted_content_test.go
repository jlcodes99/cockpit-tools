package openai

import (
	"bytes"
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdkconfig "github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/tidwall/gjson"
)

type encryptedContentRetryExecutor struct {
	mu       sync.Mutex
	payloads [][]byte
}

func (e *encryptedContentRetryExecutor) Identifier() string { return "test-provider" }

func (e *encryptedContentRetryExecutor) Execute(_ context.Context, _ *coreauth.Auth, req coreexecutor.Request, _ coreexecutor.Options) (coreexecutor.Response, error) {
	e.mu.Lock()
	e.payloads = append(e.payloads, bytes.Clone(req.Payload))
	call := len(e.payloads)
	e.mu.Unlock()
	if call == 1 {
		return coreexecutor.Response{}, websocketPinnedFailoverStatusError{
			status: http.StatusBadRequest,
			msg:    `{"error":{"type":"invalid_request_error","code":"invalid_encrypted_content","message":"expired"}}`,
		}
	}
	return coreexecutor.Response{Payload: []byte(`{"id":"resp-recovered","object":"response","output":[]}`)}, nil
}

func (e *encryptedContentRetryExecutor) ExecuteStream(_ context.Context, _ *coreauth.Auth, req coreexecutor.Request, _ coreexecutor.Options) (*coreexecutor.StreamResult, error) {
	e.mu.Lock()
	e.payloads = append(e.payloads, bytes.Clone(req.Payload))
	call := len(e.payloads)
	e.mu.Unlock()

	chunks := make(chan coreexecutor.StreamChunk, 1)
	if call == 1 {
		chunks <- coreexecutor.StreamChunk{Err: websocketPinnedFailoverStatusError{
			status: http.StatusBadRequest,
			msg:    `{"error":{"type":"invalid_request_error","code":"invalid_encrypted_content","message":"expired"}}`,
		}}
	} else {
		chunks <- coreexecutor.StreamChunk{Payload: []byte("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-recovered\",\"output\":[]}}\n\n")}
	}
	close(chunks)
	return &coreexecutor.StreamResult{Chunks: chunks}, nil
}

func (e *encryptedContentRetryExecutor) Refresh(_ context.Context, auth *coreauth.Auth) (*coreauth.Auth, error) {
	return auth, nil
}

func (e *encryptedContentRetryExecutor) CountTokens(context.Context, *coreauth.Auth, coreexecutor.Request, coreexecutor.Options) (coreexecutor.Response, error) {
	return coreexecutor.Response{}, errors.New("not implemented")
}

func (e *encryptedContentRetryExecutor) HttpRequest(context.Context, *coreauth.Auth, *http.Request) (*http.Response, error) {
	return nil, errors.New("not implemented")
}

func (e *encryptedContentRetryExecutor) Payloads() [][]byte {
	e.mu.Lock()
	defer e.mu.Unlock()
	out := make([][]byte, len(e.payloads))
	for i := range e.payloads {
		out[i] = bytes.Clone(e.payloads[i])
	}
	return out
}

func newEncryptedContentRetryTestHandler(t *testing.T) (*OpenAIResponsesAPIHandler, *encryptedContentRetryExecutor) {
	t.Helper()
	executor := &encryptedContentRetryExecutor{}
	manager := coreauth.NewManager(nil, nil, nil)
	manager.RegisterExecutor(executor)
	auth := &coreauth.Auth{ID: "auth-retry", Provider: executor.Identifier(), Status: coreauth.StatusActive}
	if _, err := manager.Register(context.Background(), auth); err != nil {
		t.Fatalf("register auth: %v", err)
	}
	registry.GetGlobalRegistry().RegisterClient(auth.ID, auth.Provider, []*registry.ModelInfo{{ID: "retry-model"}})
	t.Cleanup(func() { registry.GetGlobalRegistry().UnregisterClient(auth.ID) })
	return NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&sdkconfig.SDKConfig{}, manager)), executor
}

func TestResponsesPreservesInvalidEncryptedContentFailureNonStreaming(t *testing.T) {
	gin.SetMode(gin.TestMode)
	h, executor := newEncryptedContentRetryTestHandler(t)
	router := gin.New()
	router.POST("/v1/responses", h.Responses)
	body := `{"model":"retry-model","input":[{"type":"reasoning","id":"rs_1","encrypted_content":"expired","summary":[]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]}`
	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	router.ServeHTTP(recorder, request)

	if recorder.Code != http.StatusBadRequest || !strings.Contains(recorder.Body.String(), "invalid_encrypted_content") {
		t.Fatalf("unexpected response status=%d body=%s", recorder.Code, recorder.Body.String())
	}
	assertEncryptedContentRetryPayloads(t, executor.Payloads())
}

func TestResponsesCompactPreservesInvalidEncryptedContentFailure(t *testing.T) {
	gin.SetMode(gin.TestMode)
	h, executor := newEncryptedContentRetryTestHandler(t)
	router := gin.New()
	router.POST("/v1/responses/compact", h.Compact)
	body := `{"model":"retry-model","input":[{"type":"compaction_summary","encrypted_content":"expired","summary":[]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]}`
	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodPost, "/v1/responses/compact", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	router.ServeHTTP(recorder, request)

	if recorder.Code != http.StatusBadRequest || !strings.Contains(recorder.Body.String(), "invalid_encrypted_content") {
		t.Fatalf("unexpected compact response status=%d body=%s", recorder.Code, recorder.Body.String())
	}
	assertEncryptedContentRetryPayloads(t, executor.Payloads())
}

func TestResponsesPreservesInvalidEncryptedContentFailureBeforeFirstChunk(t *testing.T) {
	gin.SetMode(gin.TestMode)
	h, executor := newEncryptedContentRetryTestHandler(t)
	router := gin.New()
	router.POST("/v1/responses", h.Responses)
	body := `{"model":"retry-model","stream":true,"input":[{"type":"compaction","encrypted_content":"expired","summary":[]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]}`
	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	router.ServeHTTP(recorder, request)

	if recorder.Code != http.StatusBadRequest || !strings.Contains(recorder.Body.String(), "invalid_encrypted_content") {
		t.Fatalf("unexpected stream response status=%d body=%s payloads=%q", recorder.Code, recorder.Body.String(), executor.Payloads())
	}
	assertEncryptedContentRetryPayloads(t, executor.Payloads())
}

func TestResponsesWebsocketPreservesInvalidEncryptedContentFailure(t *testing.T) {
	gin.SetMode(gin.TestMode)
	h, executor := newEncryptedContentRetryTestHandler(t)
	router := gin.New()
	router.GET("/v1/responses/ws", h.ResponsesWebsocket)
	server := httptest.NewServer(router)
	defer server.Close()

	conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http")+"/v1/responses/ws", nil)
	if err != nil {
		t.Fatalf("dial websocket: %v", err)
	}
	defer func() { _ = conn.Close() }()
	if err := conn.SetReadDeadline(time.Now().Add(5 * time.Second)); err != nil {
		t.Fatal(err)
	}
	body := `{"type":"response.create","model":"retry-model","input":[{"type":"reasoning","id":"rs_1","encrypted_content":"expired","summary":[]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]}`
	if errWrite := conn.WriteMessage(websocket.TextMessage, []byte(body)); errWrite != nil {
		t.Fatalf("write websocket request: %v", errWrite)
	}
	_, payload, errRead := conn.ReadMessage()
	if errRead != nil {
		t.Fatalf("read websocket response: %v", errRead)
	}
	if !strings.Contains(string(payload), "invalid_encrypted_content") || strings.Contains(string(payload), "resp-recovered") {
		t.Fatalf("unexpected websocket payload: %s", payload)
	}
	assertEncryptedContentRetryPayloads(t, executor.Payloads())
}

func assertEncryptedContentRetryPayloads(t *testing.T, payloads [][]byte) {
	t.Helper()
	if len(payloads) != 1 {
		t.Fatalf("unexpected encrypted-content replay: attempts=%d", len(payloads))
	}
	if gjson.GetBytes(payloads[0], "input.0.encrypted_content").String() != "expired" {
		t.Fatalf("canonical history was changed: %s", payloads[0])
	}
}
