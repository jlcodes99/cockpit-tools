package main

import (
	"bufio"
	"context"
	"io"
	"net/http"
	"sync"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
)

const gatewayReloadDrainLimit = 15 * time.Minute

// The private parent pipe only stops admission. Runtime services, continuity
// and diagnostics remain alive until every accepted HTTP/WS handler finishes.
type gatewayDrain struct {
	mu        sync.Mutex
	requested chan struct{}
	closing   bool
	active    sync.WaitGroup
}

func newGatewayDrain() *gatewayDrain { return &gatewayDrain{requested: make(chan struct{})} }

func (d *gatewayDrain) request() {
	d.mu.Lock()
	defer d.mu.Unlock()
	if !d.closing {
		d.closing = true
		close(d.requested)
	}
}

func (d *gatewayDrain) control(reader io.Reader, parentClosed context.CancelFunc) {
	scan := bufio.NewScanner(reader)
	scan.Buffer(make([]byte, 128), 1024)
	for scan.Scan() {
		if scan.Text() == "drain" {
			d.request()
			return
		}
	}
	parentClosed()
}

func (d *gatewayDrain) wrap(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		d.mu.Lock()
		if d.closing {
			d.mu.Unlock()
			w.Header().Set("Connection", "close")
			w.Header().Set("Retry-After", "1")
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusServiceUnavailable)
			_, _ = io.WriteString(w, `{"error":{"type":"server_error","code":"gateway_reloading","message":"Gateway is reloading; retry the request"}}`)
			return
		}
		d.active.Add(1)
		d.mu.Unlock()
		defer d.active.Done()
		next.ServeHTTP(w, r.WithContext(handlers.WithGatewayDrain(r.Context(), d.requested)))
	})
}

func (d *gatewayDrain) finish(ctx context.Context, server *http.Server) error {
	d.request()
	// Shutdown closes the listening socket and idle connections immediately.
	// Its return does not cover hijacked sockets; active also covers WebSockets.
	shutdown := make(chan error, 1)
	go func() { shutdown <- server.Shutdown(ctx) }()
	finished := make(chan struct{})
	go func() { d.active.Wait(); close(finished) }()
	select {
	case <-finished:
		return <-shutdown
	case <-ctx.Done():
		_ = server.Close()
		return ctx.Err()
	}
}
