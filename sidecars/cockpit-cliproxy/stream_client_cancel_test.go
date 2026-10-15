package main

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func newRelayErrorTestContext(t *testing.T, ctx context.Context) (*relayServer, *gin.Context, *httptest.ResponseRecorder) {
	t.Helper()
	gin.SetMode(gin.TestMode)
	recorder := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(recorder)
	req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(`{"model":"gpt-5.6-luna","stream":true}`))
	if ctx != nil {
		req = req.WithContext(ctx)
	}
	c.Request = req
	server := &relayServer{manifest: &manifest{Locale: "zh-CN"}}
	server.bindRelayContext(c)
	return server, c, recorder
}

// 客户端主动断开时记录 499 而不是网关 5xx：宿主按状态与错误分类区分
// 「用户取消」和「网关故障」，502 + context canceled 会被判为网关取消。
func TestWriteExecutorErrorMarksClientCancelAs499(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	server, c, recorder := newRelayErrorTestContext(t, ctx)

	server.writeExecutorError(c, context.Canceled)

	if got := c.Writer.Status(); got != clientClosedRequestStatus {
		t.Fatalf("status = %d, want %d", got, clientClosedRequestStatus)
	}
	if body := recorder.Body.String(); body != "" {
		t.Fatalf("client cancel must not write a response body, got %q", body)
	}
	if got := errorCategory(c.Writer.Status(), "context canceled", false); got != "client_canceled" {
		t.Fatalf("error category = %q, want client_canceled", got)
	}
}

// 执行器重试或换号时会取消自己的 attempt context；请求上下文仍然存活时
// 不能把这种 context.Canceled 当成客户端断开。
func TestWriteExecutorErrorKeepsGatewayStatusForLiveClient(t *testing.T) {
	server, c, _ := newRelayErrorTestContext(t, context.Background())

	server.writeExecutorError(c, context.Canceled)

	if got := c.Writer.Status(); got == clientClosedRequestStatus {
		t.Fatalf("live client must not be reported as %d", clientClosedRequestStatus)
	}
	if got := c.Writer.Status(); got != http.StatusBadGateway {
		t.Fatalf("status = %d, want %d", got, http.StatusBadGateway)
	}
}

// 内置 CLIProxyAPI 判定可安全转发的重试时间必须继续传给客户端。
func TestWriteExecutorErrorForwardsSafeRetryAfter(t *testing.T) {
	server, c, recorder := newRelayErrorTestContext(t, context.Background())

	server.writeExecutorError(c, coreauth.NewHomeConcurrencyBusyError("credential concurrency limit exceeded", 30*time.Second))

	if got := recorder.Header().Get("Retry-After"); got != "30" {
		t.Fatalf("Retry-After = %q, want 30", got)
	}
	if recorder.Code != http.StatusTooManyRequests {
		t.Fatalf("status = %d, want %d", recorder.Code, http.StatusTooManyRequests)
	}
}

// 上游未给出可安全转发的重试时间时不写 Retry-After，不自行编造退避时长。
func TestWriteExecutorErrorOmitsRetryAfterWithoutSafeHeader(t *testing.T) {
	server, c, recorder := newRelayErrorTestContext(t, context.Background())

	server.writeExecutorError(c, &coreauth.Error{
		Code:       "auth_unavailable",
		Message:    "no available account",
		HTTPStatus: http.StatusServiceUnavailable,
		Retryable:  true,
	})

	if got := recorder.Header().Get("Retry-After"); got != "" {
		t.Fatalf("unexpected Retry-After header %q", got)
	}
}

// 客户端已断开时不再写 SSE 终止事件。
func TestWriteStreamTerminalErrorSkipsGoneClient(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	server, c, recorder := newRelayErrorTestContext(t, ctx)

	server.bindRelayContext(c)
	writeStreamTerminalErrorForFormat(c, errors.New("upstream stream is unavailable"), sdktranslator.FormatOpenAIResponse)

	if got := c.Writer.Status(); got != clientClosedRequestStatus {
		t.Fatalf("status = %d, want %d", got, clientClosedRequestStatus)
	}
	if body := recorder.Body.String(); body != "" {
		t.Fatalf("gone client must not receive a terminal event, got %q", body)
	}
}
