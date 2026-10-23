package helps

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestXAIClientVersionCanceledRefreshDoesNotPublish(t *testing.T) {
	restore := SetXAIClientVersionForTest("1.0.46")
	defer restore()
	ctx, cancel := context.WithCancel(context.Background())
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		cancel()
		_, _ = w.Write([]byte(`{"version":"1.2.34"}`))
	}))
	defer server.Close()
	restoreURL := OverrideXAINPMRegistryURLForTest(server.URL)
	defer restoreURL()
	refreshXAIClientVersion(ctx)
	if got := GetXAIClientVersion(); got != "1.0.46" {
		t.Fatalf("canceled refresh published %q", got)
	}
}

func TestXAIClientVersionSafetyFallsBackFromInvalidCache(t *testing.T) {
	restore := SetXAIClientVersionForTest("0.2.93")
	defer restore()
	if got := SafeXAIClientVersion("1.0.12"); got != DefaultXAIFallbackClientVersion {
		t.Fatalf("unsafe cache produced version %q", got)
	}
}
