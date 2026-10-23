package cliproxy

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestServiceXAIVersionUpdaterIsBackgroundAndCanceled(t *testing.T) {
	started, canceled := make(chan struct{}), make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(_ http.ResponseWriter, req *http.Request) {
		close(started)
		<-req.Context().Done()
		close(canceled)
	}))
	defer server.Close()
	restoreURL := helps.OverrideXAINPMRegistryURLForTest(server.URL)
	defer restoreURL()
	restoreVersion := helps.SetXAIClientVersionForTest(helps.DefaultXAIFallbackClientVersion)
	defer restoreVersion()
	finished := make(chan struct{}, 1)
	restoreHook := helps.SetXAIVersionRefreshedHookForTest(finished)
	defer restoreHook()
	service := &Service{cfg: &config.Config{}}
	stop := service.startXAIVersionUpdater(t.Context())
	defer stop()
	select {
	case <-started:
	case <-t.Context().Done():
		t.Fatal("version fetch did not start")
	}
	// Requests can use the offline version while the npm server is unresponsive.
	if got := helps.GetXAIClientVersion(); got != helps.DefaultXAIFallbackClientVersion {
		t.Fatalf("version=%q, want offline fallback", got)
	}
	stop()
	for _, done := range []<-chan struct{}{canceled, finished} {
		select {
		case <-done:
		case <-t.Context().Done():
			t.Fatal("version updater did not stop with its service")
		}
	}
}

func TestServiceXAIVersionUpdaterRespectsCanceledContext(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	service := &Service{cfg: &config.Config{}}
	stop := service.startXAIVersionUpdater(ctx)
	stop()
}
