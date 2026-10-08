package auth

import (
	"context"
	"errors"
	"net/http"
	"sync"
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
	started     chan struct{}
	release     chan struct{}
	contexts    chan context.Context
	releaseOnce sync.Once
}

func newBlockingRefreshExecutor() *blockingRefreshExecutor {
	return &blockingRefreshExecutor{
		started:  make(chan struct{}, 1),
		release:  make(chan struct{}),
		contexts: make(chan context.Context, 4),
	}
}

func (e *blockingRefreshExecutor) Identifier() string { return "fake-stop-refresh" }

func (e *blockingRefreshExecutor) Execute(context.Context, *Auth, cliproxyexecutor.Request, cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	return cliproxyexecutor.Response{}, errors.New("not implemented")
}

func (e *blockingRefreshExecutor) ExecuteStream(context.Context, *Auth, cliproxyexecutor.Request, cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	return nil, errors.New("not implemented")
}

func (e *blockingRefreshExecutor) Refresh(ctx context.Context, auth *Auth) (*Auth, error) {
	e.contexts <- ctx
	select {
	case e.started <- struct{}{}:
	default:
	}
	<-e.release
	auth.Metadata["access_token"] = "refreshed-token"
	return auth, nil
}

func (e *blockingRefreshExecutor) unblock() {
	e.releaseOnce.Do(func() { close(e.release) })
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
	store := &countingStore{}
	m := NewManager(store, nil, nil)
	exec := newBlockingRefreshExecutor()
	t.Cleanup(func() { exec.unblock(); m.StopAutoRefresh() })
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

	exec.unblock()
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
	if got := store.saveCount.Load(); got != 1 {
		t.Fatalf("cancelled refresh persisted: save count = %d, want 1 registration", got)
	}
	if current, ok := m.GetByID(a.ID); !ok || authAccessToken(current) != "tok" {
		t.Fatal("cancelled refresh replaced the current token")
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
	if err := m.persist(ctx, a); !errors.Is(err, context.Canceled) {
		t.Fatalf("persist with cancelled ctx = %v, want context.Canceled", err)
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

func newShutdownTestManager(t *testing.T) (*Manager, *blockingRefreshExecutor) {
	t.Helper()
	m := NewManager(&countingStore{}, nil, nil)
	exec := newBlockingRefreshExecutor()
	m.RegisterExecutor(exec)
	t.Cleanup(func() { exec.unblock(); m.StopAutoRefresh() })
	_, err := m.Register(context.Background(), &Auth{
		ID: "shutdown-auth", Provider: exec.Identifier(),
		Runtime: alwaysRefreshEvaluator{}, Metadata: map[string]any{"access_token": "tok"},
	})
	if err != nil {
		t.Fatal(err)
	}
	m.StartAutoRefresh(context.Background(), time.Millisecond)
	select {
	case <-exec.started:
	case <-time.After(5 * time.Second):
		t.Fatal("refresh worker did not start")
	}
	return m, exec
}

func TestStopAutoRefreshWaitsForReplacedRun(t *testing.T) {
	m, exec := newShutdownTestManager(t)
	firstCtx := <-exec.contexts
	m.StartAutoRefresh(context.Background(), time.Millisecond)
	select {
	case <-firstCtx.Done():
	case <-time.After(time.Second):
		t.Fatal("replacement did not cancel the previous run")
	}
	stopped := make(chan struct{})
	go func() { m.StopAutoRefresh(); close(stopped) }()
	select {
	case <-stopped:
		t.Fatal("stop returned before a replaced run's worker exited")
	case <-time.After(100 * time.Millisecond):
	}
	exec.unblock()
	select {
	case <-stopped:
	case <-time.After(5 * time.Second):
		t.Fatal("stop failed to finish after replaced worker exited")
	}
}

func TestConcurrentStopAutoRefreshBothWaitForWorkers(t *testing.T) {
	m, exec := newShutdownTestManager(t)
	refreshCtx := <-exec.contexts
	stopped := make(chan struct{}, 2)
	go func() { m.StopAutoRefresh(); stopped <- struct{}{} }()
	select {
	case <-refreshCtx.Done():
	case <-time.After(time.Second):
		t.Fatal("first stop did not cancel refresh")
	}
	go func() { m.StopAutoRefresh(); stopped <- struct{}{} }()
	select {
	case <-stopped:
		t.Fatal("a concurrent stop returned while a worker was still running")
	case <-time.After(100 * time.Millisecond):
	}
	exec.unblock()
	for i := 0; i < 2; i++ {
		select {
		case <-stopped:
		case <-time.After(5 * time.Second):
			t.Fatal("concurrent stop failed to finish")
		}
	}
}

func TestStartAutoRefreshDuringStopKeepsNewRun(t *testing.T) {
	m, exec := newShutdownTestManager(t)
	firstCtx := <-exec.contexts
	stopped := make(chan struct{})
	go func() { m.StopAutoRefresh(); close(stopped) }()
	select {
	case <-firstCtx.Done():
	case <-time.After(time.Second):
		t.Fatal("stop did not cancel the old run")
	}
	m.StartAutoRefresh(context.Background(), time.Millisecond)
	m.mu.RLock()
	newLoop := m.refreshLoop
	m.mu.RUnlock()
	exec.unblock()
	select {
	case <-stopped:
	case <-time.After(5 * time.Second):
		t.Fatal("stop failed to finish")
	}
	m.mu.RLock()
	active := m.refreshLoop
	m.mu.RUnlock()
	if active != newLoop || active == nil {
		t.Fatal("old run shutdown cleared the replacement run")
	}
	select {
	case <-newLoop.done:
		t.Fatal("old stop cancelled the later start")
	default:
	}
}

func TestConcurrentStartAutoRefreshLeavesLastRunActive(t *testing.T) {
	m := NewManager(nil, nil, nil)
	t.Cleanup(m.StopAutoRefresh)
	var starts sync.WaitGroup
	for i := 0; i < 16; i++ {
		starts.Go(func() { m.StartAutoRefresh(context.Background(), time.Millisecond) })
	}
	starts.Wait()
	m.mu.RLock()
	previousRuns := make([]*authAutoRefreshLoop, 0, len(m.refreshRuns))
	for loop := range m.refreshRuns {
		previousRuns = append(previousRuns, loop)
	}
	m.mu.RUnlock()
	m.StartAutoRefresh(context.Background(), time.Millisecond)
	m.mu.RLock()
	lastRun := m.refreshLoop
	m.mu.RUnlock()
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	for _, loop := range previousRuns {
		if err := loop.wait(ctx); err != nil {
			t.Fatalf("superseded run did not exit: %v", err)
		}
	}
	m.mu.RLock()
	active := m.refreshLoop
	m.mu.RUnlock()
	if active != lastRun || active == nil {
		t.Fatal("superseded run cleared the last start")
	}
	select {
	case <-lastRun.done:
		t.Fatal("last start was cancelled by an earlier start")
	default:
	}
}

func TestRefreshAuthWaitCanBeCancelled(t *testing.T) {
	m, exec := newShutdownTestManager(t)
	ctx, cancel := context.WithCancel(context.Background())
	result := make(chan error, 1)
	go func() {
		_, err := m.refreshAuthForRequest(ctx, "shutdown-auth", "tok")
		result <- err
	}()
	cancel()
	select {
	case err := <-result:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("refresh wait error = %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("cancelled refresh waited for the other in-flight refresh")
	}
	select {
	case <-exec.started:
		t.Fatal("cancelled waiter invoked the executor")
	default:
	}
}

func TestAutoRefreshWaitHasBoundedDeadline(t *testing.T) {
	loop := newAuthAutoRefreshLoop(NewManager(nil, nil, nil), time.Second, 1)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	if err := loop.wait(ctx); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("wait = %v, want deadline exceeded", err)
	}
	close(loop.done)
	if err := loop.wait(ctx); err != nil {
		t.Fatalf("finished loop wait = %v", err)
	}
}

func TestUpdateRefreshedAuthRejectsCancelledContext(t *testing.T) {
	store := &countingStore{}
	m := NewManager(store, nil, nil)
	base, err := m.Register(context.Background(), &Auth{
		ID: "cancelled-update", Provider: "codex", Metadata: map[string]any{"access_token": "old"},
	})
	if err != nil {
		t.Fatal(err)
	}
	updated := base.Clone()
	updated.Metadata["access_token"] = "new"
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := m.UpdateRefreshedAuth(ctx, base, updated); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancelled update = %v", err)
	}
	if got, ok := m.GetByID(base.ID); !ok || authAccessToken(got) != "old" {
		t.Fatal("cancelled update replaced runtime token")
	}
	if store.saveCount.Load() != 1 {
		t.Fatal("cancelled update persisted to the store")
	}
}

func TestPersistCancellationAfterLockWaitDoesNotAdvanceWatermark(t *testing.T) {
	store := &countingStore{}
	m := NewManager(store, nil, nil)
	a := &Auth{ID: "blocked-persist", RegistrationEpoch: 1, Generation: 2, Metadata: map[string]any{"access_token": "new"}}
	lock := &authPersistLock{lastEpoch: 1, lastGeneration: 1}
	m.persistLocks.Store(a.ID, lock)
	lock.mu.Lock()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- m.persist(ctx, a) }()
	cancel()
	lock.mu.Unlock()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("cancelled persist = %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("cancelled persist did not return")
	}
	if got := store.saveCount.Load(); got != 0 {
		t.Fatalf("cancelled waiter persisted %d writes", got)
	}
	if lock.lastGeneration != 1 {
		t.Fatal("cancelled waiter advanced persistence watermark")
	}
}
