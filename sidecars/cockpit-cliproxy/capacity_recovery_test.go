package main

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestCapacityFailurePreservesUpstreamHTTPAndStreamErrors(t *testing.T) {
	for _, status := range []int{http.StatusTooManyRequests, http.StatusBadGateway, http.StatusServiceUnavailable} {
		for _, code := range []string{"server_is_overloaded", "slow_down", "usage_limit_reached"} {
			err := relayStatusError{
				status:  status,
				message: `{"error":{"code":"` + code + `","message":"original upstream failure"}}`,
			}
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil)
			(&relayServer{}).writeExecutorError(c, err)
			wantCode := "upstream_error"
			if status == http.StatusTooManyRequests {
				wantCode = "rate_limited"
			}
			if w.Code != status || gjson.Get(w.Body.String(), "error.code").String() != wantCode ||
				gjson.Get(w.Body.String(), "error.message").String() != err.Error() {
				t.Fatalf("upstream failure changed: %d %s", w.Code, w.Body.String())
			}
			w = httptest.NewRecorder()
			c, _ = gin.CreateTestContext(w)
			writeStreamTerminalErrorForFormat(c, err, sdktranslator.FormatOpenAIResponse)
			if !strings.Contains(w.Body.String(), code) || strings.Contains(w.Body.String(), `"code":"server_error"`) {
				t.Fatalf("upstream stream error changed: %s", w.Body.String())
			}
		}
	}
}
