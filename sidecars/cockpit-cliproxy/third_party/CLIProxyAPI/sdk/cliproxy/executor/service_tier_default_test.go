package executor

import (
	"context"
	"sync"
	"sync/atomic"
	"testing"
)

func TestServiceTierDefaultSnapshotPreservesRequestAndRefreshesNextTurn(t *testing.T) {
	var current atomic.Value
	current.Store("priority")
	var reads atomic.Int32
	connection := WithServiceTierDefaultProvider(context.Background(), func() string { reads.Add(1); return current.Load().(string) })
	first := SnapshotServiceTierDefault(connection)
	current.Store("")
	retry := SnapshotServiceTierDefault(first)
	if tier, ok := ServiceTierDefaultSnapshot(retry); !ok || tier != "priority" {
		t.Fatalf("retry tier = %q, %v", tier, ok)
	}
	next := SnapshotServiceTierDefault(connection)
	if tier, ok := ServiceTierDefaultSnapshot(next); !ok || tier != "" {
		t.Fatalf("next tier = %q, %v", tier, ok)
	}
	if reads.Load() != 2 {
		t.Fatalf("provider read %d times", reads.Load())
	}
	if _, ok := ServiceTierDefaultSnapshot(connection); ok {
		t.Fatal("connection must not retain a request snapshot")
	}
	if _, ok := ServiceTierDefaultSnapshot(SnapshotServiceTierDefault(context.Background())); ok {
		t.Fatal("unconfigured host changed")
	}
}

func TestServiceTierDefaultConcurrentSnapshots(t *testing.T) {
	var current atomic.Value
	current.Store("priority")
	parent := WithServiceTierDefaultProvider(context.Background(), func() string { return current.Load().(string) })
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for n := 0; n < 1000; n++ {
				current.Store([]string{"", "priority"}[n%2])
				ctx := SnapshotServiceTierDefault(parent)
				tier, ok := ServiceTierDefaultSnapshot(ctx)
				if !ok || (tier != "" && tier != "priority") {
					t.Errorf("invalid snapshot %q, %v", tier, ok)
					return
				}
				again := SnapshotServiceTierDefault(ctx)
				got, _ := ServiceTierDefaultSnapshot(again)
				if got != tier {
					t.Error("snapshot changed")
				}
			}
		}()
	}
	wg.Wait()
}
