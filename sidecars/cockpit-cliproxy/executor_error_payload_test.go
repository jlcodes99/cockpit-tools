package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestRelayExecutorErrorsPreserveStructuredCauseWithoutJSONMessage(t *testing.T) {
	for _, tc := range []struct {
		code, message string
		status        int
	}{
		{"upstream_stream_error", "unexpected EOF", 502},
		{"incomplete_upstream_stream", "stream closed before response.completed", 502},
		{"request_timeout", "read tcp: i/o timeout", 408},
	} {
		for _, responseEnvelope := range []bool{false, true} {
			t.Run(fmt.Sprintf("%s/response=%v", tc.code, responseEnvelope), func(t *testing.T) {
				detail := map[string]any{"type": "server_error", "code": tc.code, "message": tc.message + "; Authorization=Bearer private-token", "param": nil, "retryable": true, "vendor": map[string]any{"request": json.Number("9007199254740993"), "access_token": "private-access"}}
				body := map[string]any{"error": detail}
				if responseEnvelope {
					body = map[string]any{"type": "response.failed", "response": body}
				}
				encoded, _ := json.Marshal(body)
				err := relayStatusError{status: tc.status, message: string(encoded)}
				for _, format := range []string{"json", "responses", "chat"} {
					w := httptest.NewRecorder()
					c, _ := gin.CreateTestContext(w)
					c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil)
					if format == "responses" {
						writeStreamTerminalErrorForFormat(c, err, sdktranslator.FormatOpenAIResponse)
					} else if format == "chat" {
						writeStreamTerminalErrorForFormat(c, err, sdktranslator.FormatOpenAI)
					} else {
						(&relayServer{}).writeExecutorError(c, err)
					}
					output := w.Body.String()
					if format != "json" {
						output = strings.TrimSpace(strings.SplitN(output, "data: ", 2)[1])
					}
					path := "error"
					if format == "responses" {
						path = "response.error"
					}
					if gjson.Get(output, path+".code").String() != tc.code || gjson.Get(output, path+".type").String() != "server_error" || !gjson.Get(output, path+".retryable").Bool() || gjson.Get(output, path+".vendor.request").Raw != "9007199254740993" {
						t.Fatalf("structured error lost: %s", output)
					}
					message := gjson.Get(output, path+".message").String()
					if !strings.HasPrefix(message, tc.message) || strings.HasPrefix(message, "{") || strings.Contains(output, "private-token") || strings.Contains(output, "private-access") {
						t.Fatalf("wrapped or unsafe error: %s", output)
					}
					if format == "json" && w.Code != tc.status {
						t.Fatalf("status = %d", w.Code)
					}
				}
			})
		}
	}
}
