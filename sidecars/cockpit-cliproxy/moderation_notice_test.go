package main

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestIsContentModerationErrorRecognizesDeepSeekRisk(t *testing.T) {
	body := `{"error":{"message":"Content Exists Risk","type":"invalid_request_error","param":null,"code":"invalid_request_error"}}`
	if !isContentModerationError(http.StatusBadRequest, body) {
		t.Fatal("DeepSeek Content Exists Risk should be treated as moderation")
	}
	if isContentModerationError(http.StatusBadRequest, `{"error":{"message":"model is required","type":"invalid_request_error"}}`) {
		t.Fatal("ordinary 400 must not be treated as moderation")
	}
	if isContentModerationError(http.StatusTooManyRequests, body) {
		t.Fatal("rate-limit status must not be rewritten as moderation")
	}
	if !isContentModerationError(http.StatusForbidden, `{"error":{"code":"content_filter","message":"blocked"}}`) {
		t.Fatal("content_filter code should be treated as moderation")
	}
}

func TestWriteExecutorErrorNormalizesContentExistsRisk(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"deepseek-flash","stream":false}`))
	server := &relayServer{manifest: &manifest{Locale: "zh-CN"}}
	server.bindRelayContext(c)

	err := relayStatusError{
		status:  http.StatusBadRequest,
		message: `{"error":{"message":"Content Exists Risk","type":"invalid_request_error","code":"invalid_request_error"}}`,
	}
	server.writeExecutorError(c, err)

	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200: %s", recorder.Code, recorder.Body.String())
	}
	if got := gjson.Get(recorder.Body.String(), "status").String(); got != "completed" {
		t.Fatalf("status field = %q, want completed: %s", got, recorder.Body.String())
	}
	if !strings.Contains(recorder.Body.String(), "本轮内容被上游内容审核拦截") {
		t.Fatalf("expected Chinese moderation notice, got %s", recorder.Body.String())
	}
	if strings.Contains(recorder.Body.String(), "Content Exists Risk") {
		t.Fatalf("upstream moderation payload leaked: %s", recorder.Body.String())
	}
}

func TestWriteStreamTerminalErrorNormalizesContentExistsRisk(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"deepseek-flash","stream":true}`))
	server := &relayServer{manifest: &manifest{Locale: "en-US"}}
	server.bindRelayContext(c)

	err := relayStatusError{
		status:  http.StatusBadRequest,
		message: `{"error":{"message":"Content Exists Risk","type":"invalid_request_error","code":"invalid_request_error"}}`,
	}
	writeStreamTerminalErrorForFormat(c, err, sdktranslator.FormatOpenAIResponse)

	body := recorder.Body.String()
	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200: %s", recorder.Code, body)
	}
	if strings.Contains(body, "response.failed") || strings.Contains(body, "Content Exists Risk") {
		t.Fatalf("stream still failed with upstream payload: %s", body)
	}
	if !strings.Contains(body, "event: response.completed") {
		t.Fatalf("expected response.completed, got %s", body)
	}
	if !strings.Contains(body, "blocked by upstream content moderation") {
		t.Fatalf("expected English moderation notice, got %s", body)
	}
}

func TestWriteExecutorErrorLeavesOrdinary400Unchanged(t *testing.T) {
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"deepseek-flash"}`))
	server := &relayServer{}
	err := relayStatusError{status: http.StatusBadRequest, message: "model is required"}
	server.writeExecutorError(c, err)
	if recorder.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400: %s", recorder.Code, recorder.Body.String())
	}
	if !strings.Contains(recorder.Body.String(), "model is required") {
		t.Fatalf("ordinary 400 body changed: %s", recorder.Body.String())
	}
}

func TestProviderGatewayNormalizesContentExistsRiskStream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusBadRequest)
		_, _ = io.WriteString(w, `{"error":{"message":"Content Exists Risk","type":"invalid_request_error","code":"invalid_request_error"}}`)
	}))
	t.Cleanup(upstream.Close)

	server := &relayServer{manifest: &manifest{Locale: "zh-CN"}}
	requestBody := `{"model":"deepseek-flash","stream":true,"input":[{"role":"user","content":[{"type":"input_text","text":"hi"}]}]}`
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(requestBody))
	request.Header.Set("Content-Type", "application/json")
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = request
	server.bindRelayContext(c)

	server.handleProviderGatewayRequest(c, &providerGatewaySpec{
		BaseURL:       upstream.URL + "/v1",
		APIKey:        "sk-test",
		UpstreamModel: "deepseek-flash",
		WireAPI:       "chat_completions",
	}, []byte(requestBody), "deepseek-flash", sdktranslator.FormatOpenAIResponse, "")

	if recorder.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200: %s", recorder.Code, recorder.Body.String())
	}
	body := recorder.Body.String()
	if strings.Contains(body, "Content Exists Risk") || strings.Contains(body, `"type":"invalid_request_error"`) {
		t.Fatalf("upstream 400 leaked: %s", body)
	}
	if !strings.Contains(body, "event: response.completed") || !strings.Contains(body, "本轮内容被上游内容审核拦截") {
		t.Fatalf("expected completed moderation notice, got %s", body)
	}
}

func TestProviderGatewayLeavesOrdinary400Unchanged(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusBadRequest)
		_, _ = io.WriteString(w, `{"error":{"message":"model is required","type":"invalid_request_error"}}`)
	}))
	t.Cleanup(upstream.Close)

	server := &relayServer{}
	requestBody := `{"model":"deepseek-flash","stream":false}`
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(requestBody))
	request.Header.Set("Content-Type", "application/json")
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	c.Request = request

	server.handleProviderGatewayRequest(c, &providerGatewaySpec{
		BaseURL:       upstream.URL + "/v1",
		APIKey:        "sk-test",
		UpstreamModel: "deepseek-flash",
		WireAPI:       "chat_completions",
	}, []byte(requestBody), "deepseek-flash", sdktranslator.FormatOpenAIResponse, "")

	if recorder.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400: %s", recorder.Code, recorder.Body.String())
	}
	if !strings.Contains(recorder.Body.String(), "model is required") {
		t.Fatalf("ordinary 400 body changed: %s", recorder.Body.String())
	}
}
