package auth

import (
	"context"
	"errors"
	"net/http"
	"testing"
	"time"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// alwaysRefreshEvaluator makes every auth immediately due for refresh, so the
// auto-refresh worker dispatches a job without waiting for token expiry.
type alwaysRefreshEvaluator struct{}

func (alwaysRefreshEvaluator) ShouldRefresh(time.Time, *Auth) bool { return true }

// blockingRefreshExecutor stalls inside Refresh until the test releases it,
// giving a deterministic in-flight worker for shutdown assertions.
type blockingRefreshExecutor struct {
	started chan struct{}
	release chan struct{}
}

func newBlockingRefreshExecutor() *blockingRefreshExecutor {
	return &blockingRefreshExecutor{
		started: make(chan struct{}, 1),
		release: make(chan struct{}),
	}
}

func (e *blockingRefreshExecutor) Identifier() string { return "fake-stop-refresh" }

func (e *blockingRefreshExecutor) Execute(context.Context, *Auth, cliproxyexecutor.Request, cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	return cliproxyexecutor.Response{}, errors.New("not implemented")
}

func (e *blockingRefreshExecutor) ExecuteStream(context.Context, *Auth, cliproxyexecutor.Request, cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	return nil, errors.New("not implemented")
}

func (e *blockingRefreshExecutor) Refresh(_ context.Context, auth *Auth) (*Auth, error) {
	select {
	case e.started <- struct{}{}:
	default:
	}
	<-e.release
	return auth, nil
}

func (e *blockingRefreshExecutor) CountTokens(context.Context, *Auth, cliproxyexecutor.Request, cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	return cliproxyexecutor.Response{}, errors.New("not implemented")
}

func (e *blockingRefreshExecutor) HttpRequest(context.Context, *Auth, *http.Request) (*http.Response, error) {
	return nil, errors.New("not implemented")
}

// TestStopAutoRefreshWaitsForInFlightWorker is the regression test for refresh
// writes landing after shutdown: previously StopAutoRefresh cancelled the loop
// but returned immediately, so a worker mid-Refresh could still persist into
// an auth directory the caller had already begun tearing down.
func TestStopAutoRefreshWaitsForInFlightWorker(t *testing.T) {
	m := NewManager(&countingStore{}, nil, nil)
	exec := newBlockingRefreshExecutor()
	m.RegisterExecutor(exec)

	a := &Auth{
		ID:       "in-flight-auth",
		Provider: "fake-stop-refresh",
		Runtime:  alwaysRefreshEvaluator{},
		Metadata: map[string]any{"access_token": "tok"},
	}
	if _, err := m.Register(context.Background(), a); err != nil {
		t.Fatalf("register auth: %v", err)
	}

	m.StartAutoRefresh(context.Background(), time.Millisecond)
	m.mu.RLock()
	loop := m.refreshLoop
	m.mu.RUnlock()
	if loop == nil {
		t.Fatal("refresh loop not running after StartAutoRefresh")
	}

	select {
	case <-exec.started:
	case <-time.After(10 * time.Second):
		t.Fatal("refresh worker never picked up the auth")
	}

	stopped := make(chan struct{})
	go func() {
		m.StopAutoRefresh()
		close(stopped)
	}()

	select {
	case <-stopped:
		t.Fatal("StopAutoRefresh returned while a refresh worker was still in flight")
	case <-time.After(200 * time.Millisecond):
	}

	close(exec.release)
	select {
	case <-stopped:
	case <-time.After(10 * time.Second):
		t.Fatal("StopAutoRefresh did not return after the worker finished")
	}

	// The loop must be fully exited: no worker can write afterwards.
	select {
	case <-loop.done:
	case <-time.After(10 * time.Second):
		t.Fatal("refresh loop did not exit after StopAutoRefresh returned")
	}
}

// TestPersistSkipsWriteWhenContextCancelled pins the guard that keeps a
// cancelled caller (e.g., a refresh worker during shutdown) from starting a
// durable write.
func TestPersistSkipsWriteWhenContextCancelled(t *testing.T) {
	store := &countingStore{}
	m := NewManager(store, nil, nil)
	a := &Auth{
		ID:       "persist-auth",
		Provider: "fake-stop-refresh",
		Metadata: map[string]any{"access_token": "tok"},
	}

	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := m.persist(ctx, a); err != nil {
		t.Fatalf("persist with cancelled ctx: %v", err)
	}
	if got := store.saveCount.Load(); got != 0 {
		t.Fatalf("expected no save on cancelled context, got %d", got)
	}

	if err := m.persist(context.Background(), a); err != nil {
		t.Fatalf("persist with live ctx: %v", err)
	}
	if got := store.saveCount.Load(); got != 1 {
		t.Fatalf("expected one save on live context, got %d", got)
	}
}
