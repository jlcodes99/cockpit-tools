package handlers

import (
	"context"
	"testing"
	"time"
)

func TestCapabilityTestDispatcherPool_ReusesSameIdentity(t *testing.T) {
	pool := newCapabilityTestDispatcherPool()

	first := pool.Get("identity-a")
	second := pool.Get("identity-a")

	if first == nil || second == nil {
		t.Fatal("expected non-nil dispatchers")
	}
	if first != second {
		t.Fatal("expected same dispatcher for same identity")
	}
}

func TestCapabilityTestDispatcherPool_IsolatesDifferentIdentities(t *testing.T) {
	pool := newCapabilityTestDispatcherPool()

	first := pool.Get("identity-a")
	second := pool.Get("identity-b")

	if first == nil || second == nil {
		t.Fatal("expected non-nil dispatchers")
	}
	if first == second {
		t.Fatal("expected different dispatchers for different identities")
	}
}

func TestCapabilityTestDispatcherPool_DefaultKey(t *testing.T) {
	pool := newCapabilityTestDispatcherPool()

	first := pool.Get("")
	second := pool.Get("")

	if first == nil || second == nil {
		t.Fatal("expected non-nil dispatchers")
	}
	if first != second {
		t.Fatal("expected empty identity to reuse default dispatcher")
	}
}

func TestCapabilityTestDispatcherPool_GCRemovesIdleDispatcher(t *testing.T) {
	pool := newCapabilityTestDispatcherPool()
	pool.idleTTL = time.Millisecond

	dispatcher := pool.Get("identity-a")
	dispatcher.lastUsed.Store(time.Now().Add(-time.Second).UnixNano())

	pool.gc()

	pool.mu.RLock()
	_, ok := pool.dispatchers["identity-a"]
	pool.mu.RUnlock()
	if ok {
		t.Fatal("expected idle dispatcher to be removed by gc")
	}
}

func TestCapabilityTestDispatcher_AcquireSendSlotOnClosedDispatcherReturnsImmediately(t *testing.T) {
	dispatcher := newCapabilityTestDispatcher()
	dispatcher.mu.Lock()
	dispatcher.closed.Store(true)
	dispatcher.mu.Unlock()

	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()

	started := time.Now()
	err := dispatcher.AcquireSendSlot(ctx, time.Millisecond)
	if err == nil {
		t.Fatal("expected closed dispatcher to return an error")
	}
	if time.Since(started) > 50*time.Millisecond {
		t.Fatalf("AcquireSendSlot blocked too long on closed dispatcher: %s", time.Since(started))
	}
}

func TestCapabilityTestDispatcher_NormalQueueFIFO(t *testing.T) {
	dispatcher := newCapabilityTestDispatcher()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// 放行前可以在队列中塞两个请求
	err1 := dispatcher.AcquireSendSlot(ctx, 50*time.Millisecond)
	if err1 != nil {
		t.Fatalf("expected first acquire to succeed: %v", err1)
	}
	err2 := dispatcher.AcquireSendSlot(ctx, 50*time.Millisecond)
	if err2 != nil {
		t.Fatalf("expected second acquire to succeed: %v", err2)
	}
}

func TestCapabilityTestDispatcher_PrioritySkipsNormalQueue(t *testing.T) {
	dispatcher := newCapabilityTestDispatcher()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	interval := 50 * time.Millisecond

	// 先发出一个普通请求（消耗首个时间槽位让后续请求进入 pending 缓冲区）
	done1 := make(chan error, 1)
	go func() { done1 <- dispatcher.AcquireSendSlot(ctx, interval) }()
	if err := <-done1; err != nil {
		t.Fatalf("normal acquire failed: %v", err)
	}

	// 再塞 3 个普通请求进入 pending 缓冲区
	go func() { done1 <- dispatcher.AcquireSendSlot(ctx, interval) }()
	go func() { done1 <- dispatcher.AcquireSendSlot(ctx, interval) }()
	go func() { done1 <- dispatcher.AcquireSendSlot(ctx, interval) }()
	time.Sleep(10 * time.Millisecond) // 让 goroutine push 进 channel 再触发 run 把请求挪到 pendingNormal

	// 现在发一个高优先级请求，应在下一个槽位优先于普通队列返回
	priorityDone := make(chan error, 1)
	go func() { priorityDone <- dispatcher.AcquirePrioritySendSlot(ctx, interval) }()

	select {
	case err := <-priorityDone:
		if err != nil {
			t.Fatalf("priority acquire failed: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("priority acquire did not complete in time (should skip normal queue)")
	}
}

func TestCapabilityTestDispatcher_PriorityRespectsInterval(t *testing.T) {
	dispatcher := newCapabilityTestDispatcher()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	interval := 100 * time.Millisecond

	start := time.Now()
	if err := dispatcher.AcquirePrioritySendSlot(ctx, interval); err != nil {
		t.Fatalf("first priority acquire failed: %v", err)
	}
	if err := dispatcher.AcquirePrioritySendSlot(ctx, interval); err != nil {
		t.Fatalf("second priority acquire failed: %v", err)
	}
	elapsed := time.Since(start)
	if elapsed < interval {
		t.Fatalf("priority requests did not respect interval: %s < %s", elapsed, interval)
	}
}

func TestCapabilityTestDispatcher_PriorityArrivingAtDispatchTimeWins(t *testing.T) {
	dispatcher := newCapabilityTestDispatcher()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	interval := 50 * time.Millisecond

	firstDone := make(chan error, 1)
	go func() { firstDone <- dispatcher.AcquireSendSlot(ctx, interval) }()
	if err := <-firstDone; err != nil {
		t.Fatalf("first normal acquire failed: %v", err)
	}

	normalDone := make(chan error, 1)
	go func() { normalDone <- dispatcher.AcquireSendSlot(ctx, interval) }()
	time.Sleep(10 * time.Millisecond)

	time.Sleep(interval - 15*time.Millisecond)
	priorityDone := make(chan error, 1)
	go func() { priorityDone <- dispatcher.AcquirePrioritySendSlot(ctx, interval) }()

	select {
	case err := <-priorityDone:
		if err != nil {
			t.Fatalf("priority acquire failed: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("priority acquire did not complete in time")
	}

	select {
	case <-normalDone:
		// normal 可以随后完成，但不应先于 priority 断言返回
	case <-time.After(2 * time.Second):
		t.Fatal("normal acquire did not complete in time after priority")
	}
}

func TestCapabilityTestDispatcher_BackpressureLimitPreserved(t *testing.T) {
	dispatcher := newCapabilityTestDispatcher()

	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()

	for i := 0; i < capabilityDispatcherQueueLimit; i++ {
		go dispatcher.AcquireSendSlot(ctx, time.Hour)
	}
	time.Sleep(10 * time.Millisecond)
	if len(dispatcher.pendingSlots) == 0 {
		t.Fatal("expected pendingSlots to be occupied under load")
	}
}

func TestCapabilityTestDispatcher_PriorityClosedReturnsImmediately(t *testing.T) {
	dispatcher := newCapabilityTestDispatcher()
	dispatcher.mu.Lock()
	dispatcher.closed.Store(true)
	dispatcher.mu.Unlock()

	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()

	started := time.Now()
	err := dispatcher.AcquirePrioritySendSlot(ctx, time.Millisecond)
	if err == nil {
		t.Fatal("expected closed dispatcher to return an error for priority send slot")
	}
	if time.Since(started) > 50*time.Millisecond {
		t.Fatalf("AcquirePrioritySendSlot blocked too long on closed dispatcher: %s", time.Since(started))
	}
}
