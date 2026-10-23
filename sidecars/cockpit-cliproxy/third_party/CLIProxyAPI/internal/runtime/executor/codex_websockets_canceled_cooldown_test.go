package executor

import (
	"context"
	"net/http"
	"sync"
	"testing"
	"time"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// End is the deterministic barrier between invalidation and delivery of the failure.
type cooldownDeliveryLifecycle struct {
	entered chan struct{}
	release chan struct{}
	once    sync.Once
}

func (*cooldownDeliveryLifecycle) Bind(func() error) error { return nil }
func (l *cooldownDeliveryLifecycle) End(string) {
	l.once.Do(func() { close(l.entered); <-l.release })
}

func TestCodexWebsocketQuotaErrorDeliveredBeforeDisconnectCancellation(t *testing.T) {
	for _, frame := range []string{
		`{"type":"error","status":429,"error":{"type":"usage_limit_reached","message":"usage limit reached","resets_in_seconds":600}}`,
		`{"type":"response.failed","response":{"error":{"type":"usage_limit_reached","status_code":429,"message":"usage limit reached","resets_in_seconds":600}}}`,
	} {
		t.Run(frame, func(t *testing.T) {
			server := codexWebsocketServerHoldingConnection(t, frame)
			defer server.Close()
			exec := NewCodexWebsocketsExecutor(codexBufferingConfig(false))
			exec.store = &codexWebsocketSessionStore{sessions: make(map[string]*codexWebsocketSession)}
			const session = "quota-delivery"
			disconnect := exec.UpstreamDisconnectChan(session)
			lifecycle := &cooldownDeliveryLifecycle{entered: make(chan struct{}), release: make(chan struct{})}
			var releaseOnce sync.Once
			release := func() { releaseOnce.Do(func() { close(lifecycle.release) }) }
			defer release()
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			req, opts := codexWebsocketRequest()
			opts.ExecutionLifecycle = lifecycle
			opts.Metadata = map[string]any{cliproxyexecutor.ExecutionSessionMetadataKey: session}
			result, err := exec.ExecuteStream(ctx, codexTestAuth(server.URL), req, opts)
			if err != nil {
				t.Fatal(err)
			}
			select {
			case <-lifecycle.entered:
			case <-time.After(5 * time.Second):
				t.Fatal("failure did not invalidate its socket")
			}
			// An early notification cannot hide behind goroutine scheduling: End has already been entered.
			select {
			case <-disconnect:
				cancel()
				t.Fatal("disconnect preceded error delivery")
			default:
			}
			release()
			var streamErr error
			for chunk := range result.Chunks {
				if chunk.Err != nil {
					streamErr = chunk.Err
				}
			}
			if streamErr == nil || statusCodeFromTestError(t, streamErr) != http.StatusTooManyRequests {
				t.Fatalf("lost quota failure: %v", streamErr)
			}
			select {
			case <-disconnect:
				cancel()
			case <-time.After(5 * time.Second):
				t.Fatal("disconnect was not signalled after delivery")
			}
		})
	}
}
