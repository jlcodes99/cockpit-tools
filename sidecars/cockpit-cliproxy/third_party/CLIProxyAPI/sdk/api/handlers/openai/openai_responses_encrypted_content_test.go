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

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/interfaces"
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

func TestSanitizeInvalidEncryptedContentRetryBody(t *testing.T) {
	body := []byte(`{
		"model":"gpt-5.5",
		"sequence":900719925474099312345,
		"input":[
			{"type":"compaction","encrypted_content":"expired","summary":[{"text":"drop"}]},
			{"type":"compaction_summary","summary":[{"text":"keep"}]},
			{"type":"reasoning","id":"rs_1","encrypted_content":"expired","content":null,"summary":[]},
			{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}
		]
	}`)

	got, changed := sanitizeInvalidEncryptedContentRetryBody(body)
	if !changed {
		t.Fatal("expected encrypted replay content to be sanitized")
	}
	if value := gjson.GetBytes(got, "sequence").Raw; value != "900719925474099312345" {
		t.Fatalf("large integer = %s, want exact original value", value)
	}
	if value := gjson.GetBytes(got, "input.#").Int(); value != 3 {
		t.Fatalf("input length = %d, want 3: %s", value, got)
	}
	if value := gjson.GetBytes(got, "input.0.type").String(); value != "compaction_summary" {
		t.Fatalf("unencrypted compaction type = %q, want compaction_summary", value)
	}
	if gjson.GetBytes(got, "input.1.encrypted_content").Exists() || gjson.GetBytes(got, "input.1.content").Exists() {
		t.Fatalf("reasoning encrypted/null content was not removed: %s", got)
	}
	if value := gjson.GetBytes(got, "input.1.id").String(); value != "rs_1" {
		t.Fatalf("reasoning id = %q, want rs_1", value)
	}
}

func TestSanitizeInvalidEncryptedContentRetryBodyNoChange(t *testing.T) {
	body := []byte(`{"input":[{"type":"reasoning","summary":[]},{"type":"compaction","summary":[]}]}`)
	got, changed := sanitizeInvalidEncryptedContentRetryBody(body)
	if changed {
		t.Fatalf("unexpected change: %s", got)
	}
}

func TestIsInvalidEncryptedContentError(t *testing.T) {
	errMsg := &interfaces.ErrorMessage{StatusCode: 400, Error: errors.New(`{"error":{"code":"invalid_encrypted_content"}}`)}
	if !isInvalidEncryptedContentError(errMsg) {
		t.Fatal("expected invalid_encrypted_content error to match")
	}
}

func TestResponsesRetriesInvalidEncryptedContentNonStreaming(t *testing.T) {
	gin.SetMode(gin.TestMode)
	h, executor := newEncryptedContentRetryTestHandler(t)
	router := gin.New()
	router.POST("/v1/responses", h.Responses)
	body := `{"model":"retry-model","input":[{"type":"reasoning","id":"rs_1","encrypted_content":"expired","summary":[]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]}`
	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	router.ServeHTTP(recorder, request)

	if recorder.Code != http.StatusOK || gjson.Get(recorder.Body.String(), "id").String() != "resp-recovered" {
		t.Fatalf("unexpected response status=%d body=%s", recorder.Code, recorder.Body.String())
	}
	assertEncryptedContentRetryPayloads(t, executor.Payloads())
}

func TestResponsesCompactRetriesInvalidEncryptedContent(t *testing.T) {
	gin.SetMode(gin.TestMode)
	h, executor := newEncryptedContentRetryTestHandler(t)
	router := gin.New()
	router.POST("/v1/responses/compact", h.Compact)
	body := `{"model":"retry-model","input":[{"type":"compaction_summary","encrypted_content":"expired","summary":[]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]}`
	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodPost, "/v1/responses/compact", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	router.ServeHTTP(recorder, request)

	if recorder.Code != http.StatusOK || gjson.Get(recorder.Body.String(), "id").String() != "resp-recovered" {
		t.Fatalf("unexpected compact response status=%d body=%s", recorder.Code, recorder.Body.String())
	}
	assertEncryptedContentRetryPayloads(t, executor.Payloads())
}

func TestResponsesRetriesInvalidEncryptedContentBeforeFirstStreamChunk(t *testing.T) {
	gin.SetMode(gin.TestMode)
	h, executor := newEncryptedContentRetryTestHandler(t)
	router := gin.New()
	router.POST("/v1/responses", h.Responses)
	body := `{"model":"retry-model","stream":true,"input":[{"type":"compaction","encrypted_content":"expired","summary":[]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]}`
	recorder := httptest.NewRecorder()
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	router.ServeHTTP(recorder, request)

	if recorder.Code != http.StatusOK || !strings.Contains(recorder.Body.String(), "resp-recovered") {
		t.Fatalf("unexpected stream response status=%d body=%s payloads=%q", recorder.Code, recorder.Body.String(), executor.Payloads())
	}
	assertEncryptedContentRetryPayloads(t, executor.Payloads())
}

func TestResponsesWebsocketRetriesInvalidEncryptedContentOnce(t *testing.T) {
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
	body := `{"type":"response.create","model":"retry-model","input":[{"type":"reasoning","id":"rs_1","encrypted_content":"expired","summary":[]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]}`
	if errWrite := conn.WriteMessage(websocket.TextMessage, []byte(body)); errWrite != nil {
		t.Fatalf("write websocket request: %v", errWrite)
	}
	_, payload, errRead := conn.ReadMessage()
	if errRead != nil {
		t.Fatalf("read websocket response: %v", errRead)
	}
	if gjson.GetBytes(payload, "type").String() != "response.completed" {
		t.Fatalf("unexpected websocket payload: %s", payload)
	}
	assertEncryptedContentRetryPayloads(t, executor.Payloads())
}

func assertEncryptedContentRetryPayloads(t *testing.T, payloads [][]byte) {
	t.Helper()
	if len(payloads) != 2 {
		t.Fatalf("attempt count = %d, want 2", len(payloads))
	}
	if !gjson.GetBytes(payloads[0], "input.0.encrypted_content").Exists() {
		t.Fatalf("first request lost encrypted content: %s", payloads[0])
	}
	if gjson.GetBytes(payloads[1], "input.0.encrypted_content").Exists() {
		t.Fatalf("retry still contains encrypted content: %s", payloads[1])
	}
}
