package helps

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
)

func TestRecordAPIResponseMetadataStoresHeadersWhenRequestLogDisabled(t *testing.T) {
	ctx := logging.WithResponseHeadersHolder(context.Background())
	headers := http.Header{}
	headers.Add("X-Upstream-Request-Id", "upstream-req-1")

	RecordAPIResponseMetadata(ctx, &config.Config{}, http.StatusOK, headers)
	headers.Set("X-Upstream-Request-Id", "mutated")

	got := logging.GetResponseHeaders(ctx)
	if got.Get("X-Upstream-Request-Id") != "upstream-req-1" {
		t.Fatalf("response header = %q, want %q", got.Get("X-Upstream-Request-Id"), "upstream-req-1")
	}
}

func TestRecordAPIStreamSemanticStatusKeepsTransportStatusSeparate(t *testing.T) {
	gin.SetMode(gin.TestMode)
	ginCtx, _ := gin.CreateTestContext(httptest.NewRecorder())
	ctx := context.WithValue(context.Background(), "gin", ginCtx)
	cfg := &config.Config{}
	cfg.RequestLog = true

	RecordAPIResponseMetadata(ctx, cfg, http.StatusOK, http.Header{})
	RecordAPIStreamSemanticStatus(ctx, cfg, http.StatusServiceUnavailable, "response.failed")

	raw, exists := ginCtx.Get(apiResponseKey)
	if !exists {
		t.Fatal("expected aggregated API response log")
	}
	logged := string(raw.([]byte))
	for _, want := range []string{"Status: 200", "Stream-Status: 503", "Stream-Event: response.failed"} {
		if !strings.Contains(logged, want) {
			t.Fatalf("response log missing %q: %s", want, logged)
		}
	}
}
