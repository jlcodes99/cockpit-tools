package openai

import (
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/interfaces"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	sdkconfig "github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/tidwall/gjson"
)

func TestResponsesTransportErrorRetainsCodeAndSanitizesAllFinalFormats(t *testing.T) {
	for _, errorBody := range []string{
		`{"error":{"type":"server_error","code":"upstream_stream_error","message":"unexpected EOF; Authorization=Bearer private-token; refresh_token=private-refresh"}}`,
		`{"error":{"type":"server_error","code":"incomplete_upstream_stream","message":"stream closed before response.completed"}}`,
		`{"error":{"type":"server_error","code":"request_timeout","message":"read tcp: i/o timeout"}}`,
		`{"error":{"type":"server_error","code":"upstream_business_failure","message":"upstream rejected request; Authorization=Bearer private-token"}}`,
		`{"error":{"type":"server_error","code":"upstream_stream_error","message":"unexpected EOF; token='private secret'; password:'foo bar'; authorization: Basic Zm9vOmJhcg==; url=https://user:pass@local.test/?token=private"}}`,
	} {
		t.Run(errorBody, func(t *testing.T) {
			errs := make(chan *interfaces.ErrorMessage, 1)
			errs <- &interfaces.ErrorMessage{StatusCode: http.StatusBadGateway, Error: errors.New(errorBody)}
			close(errs)
			recorder := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(recorder)
			c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil)
			c.Request.Header.Set("User-Agent", "Codex Desktop/26.803.41515")
			handler := NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&sdkconfig.SDKConfig{}, nil))
			handler.forwardResponsesStream(c, recorder, func(error) {}, make(chan []byte), errs, &responsesSSEFramer{})
			if strings.Count(recorder.Body.String(), "event: response.failed") != 1 {
				t.Fatalf("Codex needs one response.failed: %s", recorder.Body.String())
			}
			wsPayload, err := buildResponsesWebsocketErrorPayload(&interfaces.ErrorMessage{StatusCode: 502, Error: errors.New(errorBody)})
			if err != nil {
				t.Fatal(err)
			}
			for _, output := range []string{recorder.Body.String(), string(wsPayload)} {
				for _, secret := range []string{"private-token", "private-refresh", "private secret", "foo bar", "Zm9vOmJhcg", "user:pass", "local.test", "stream error:", "stream disconnected before completion:"} {
					if strings.Contains(output, secret) {
						t.Fatalf("unsafe or duplicated final error: %s", output)
					}
				}
				if !strings.Contains(output, strings.Split(strings.Split(errorBody, `"code":"`)[1], `"`)[0]) {
					t.Fatalf("upstream/local error code lost: %s", output)
				}
			}
		})
	}
}

func TestResponsesWebsocketClassifiedReadFailureBeforeCloseAndReconnect(t *testing.T) {
	for _, tc := range []struct {
		code, message string
		status        int
	}{
		{"upstream_stream_error", "unexpected EOF; Authorization=Bearer private-token", 502},
		{"incomplete_upstream_stream", "stream closed before response.completed", 502},
		{"request_timeout", "read tcp: i/o timeout", 408},
	} {
		t.Run(tc.code, func(t *testing.T) {
			var attempts atomic.Int32
			done := make(chan error, 2)
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				conn, err := responsesWebsocketUpgrader.Upgrade(w, r, nil)
				if err != nil {
					done <- err
					return
				}
				defer conn.Close()
				_, request, err := conn.ReadMessage()
				if err != nil || !strings.Contains(string(request), "full context") || gjson.GetBytes(request, "previous_response_id").Exists() {
					done <- fmt.Errorf("retry did not send full context: %s %v", request, err)
					return
				}
				if attempts.Add(1) == 1 {
					errMsg := &interfaces.ErrorMessage{StatusCode: tc.status, Error: errors.New(fmt.Sprintf(`{"error":{"type":"server_error","code":%q,"message":%q}}`, tc.code, tc.message))}
					_, wrote, err := writeResponsesWebsocketTerminalError(newResponsesWebsocketWriter(conn), nil, errMsg, nil)
					if !wrote || !errors.Is(err, websocket.ErrCloseSent) {
						done <- fmt.Errorf("classified failure was suppressed: wrote=%v error=%v", wrote, err)
						return
					}
					done <- nil
					return
				}
				done <- conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.completed","response":{"status":"completed"}}`))
			}))
			defer server.Close()
			for attempt := 0; attempt < 2; attempt++ {
				conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http"), nil)
				if err != nil {
					t.Fatal(err)
				}
				if err := conn.SetReadDeadline(time.Now().Add(3 * time.Second)); err != nil {
					t.Fatal(err)
				}
				if err := conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"response.create","input":[{"role":"user","content":"full context"}]}`)); err != nil {
					t.Fatal(err)
				}
				_, payload, err := conn.ReadMessage()
				if err != nil {
					t.Fatal(err)
				}
				if attempt == 0 {
					if gjson.GetBytes(payload, "type").String() != "error" || gjson.GetBytes(payload, "error.code").String() != tc.code || int(gjson.GetBytes(payload, "status").Int()) != tc.status {
						t.Fatalf("classified failure was lost: %s", payload)
					}
					if strings.Contains(string(payload), "private-token") || strings.Contains(string(payload), "stream error:") || !strings.Contains(string(payload), strings.Split(tc.message, ";")[0]) {
						t.Fatalf("unsafe, duplicate or missing cause: %s", payload)
					}
					if _, _, err := conn.ReadMessage(); err == nil {
						t.Fatal("failed upstream connection stayed open")
					}
				} else if gjson.GetBytes(payload, "type").String() != "response.completed" {
					t.Fatalf("fresh connection retry failed: %s", payload)
				}
				_ = conn.Close()
				if err := <-done; err != nil {
					t.Fatal(err)
				}
			}
		})
	}
}
