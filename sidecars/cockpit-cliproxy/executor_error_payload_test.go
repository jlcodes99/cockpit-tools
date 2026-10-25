package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestRelaySelectionErrorsKeepTheirCodeAcrossJSONAndSSE(t *testing.T) {
	for _, code := range []string{"auth_not_found", "auth_unavailable"} {
		for _, format := range []string{"json", "responses", "chat"} {
			t.Run(code+"/"+format, func(t *testing.T) {
				err := fmt.Errorf("selection: %w", &coreauth.Error{
					Code: code, Message: "No eligible account; Authorization=Bearer private-token",
					HTTPStatus: http.StatusServiceUnavailable,
				})
				w := httptest.NewRecorder()
				c, _ := gin.CreateTestContext(w)
				c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil)
				switch format {
				case "responses":
					writeStreamTerminalErrorForFormat(c, err, sdktranslator.FormatOpenAIResponse)
				case "chat":
					writeStreamTerminalErrorForFormat(c, err, sdktranslator.FormatOpenAI)
				default:
					(&relayServer{}).writeExecutorError(c, err)
				}
				body := w.Body.String()
				if format != "json" {
					body = strings.TrimSpace(strings.SplitN(body, "data: ", 2)[1])
				}
				path := "error"
				if format == "responses" {
					path = "response.error"
				}
				if gjson.Get(body, path+".code").String() != code || strings.Contains(body, "private-token") {
					t.Fatalf("selection cause lost or credential leaked: %s", body)
				}
				if format == "json" && w.Code != http.StatusServiceUnavailable {
					t.Fatalf("selection status changed: %d", w.Code)
				}
			})
		}
	}
}

func TestSelectionFailuresAreNotClassifiedAsUpstreamFailures(t *testing.T) {
	for _, body := range []string{
		`{"error":{"code":"auth_not_found","message":"no eligible account"}}`,
		`{"response":{"error":{"code":"auth_unavailable","message":"no eligible account"}}}`,
		"Error #01: auth_not_found: no eligible account",
		"auth_unavailable: no eligible account",
	} {
		if got := errorCategory(http.StatusServiceUnavailable, body, false); got != "no_available_account" {
			t.Fatalf("selection failure classified as %q: %s", got, body)
		}
	}
	if got := errorCategory(http.StatusServiceUnavailable, errors.New("upstream is unavailable").Error(), false); got != "upstream_error" {
		t.Fatalf("unrelated upstream failure changed category: %s", got)
	}
}

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
