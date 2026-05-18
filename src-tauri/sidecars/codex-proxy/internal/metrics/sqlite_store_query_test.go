package metrics

import (
	"path/filepath"
	"testing"
	"time"
)

func TestQueryAggregatedHistoryWaitsForFlushAndFlushesBuffer(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 30,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() error = %v", err)
	}
	defer store.Close()

	record := PersistentRecord{
		MetricsKey:  GenerateMetricsKey("https://example.com", "sk-test"),
		BaseURL:     "https://example.com",
		KeyMask:     "sk-***",
		Timestamp:   time.Now(),
		Success:     true,
		APIType:     "messages",
		InputTokens: 10,
	}

	store.bufferMu.Lock()
	store.writeBuffer = append(store.writeBuffer, record)
	store.bufferMu.Unlock()

	store.flushMu.Lock()
	defer store.flushMu.Unlock()

	resultCh := make(chan []AggregatedBucket, 1)
	errCh := make(chan error, 1)
	go func() {
		buckets, err := store.QueryAggregatedHistory("messages", time.Now().Add(-time.Hour), 60, record.MetricsKey, "")
		if err != nil {
			errCh <- err
			return
		}
		resultCh <- buckets
	}()

	select {
	case <-resultCh:
		t.Fatal("QueryAggregatedHistory() should wait for flushMu, but returned early")
	case err := <-errCh:
		t.Fatalf("QueryAggregatedHistory() unexpected error: %v", err)
	case <-time.After(100 * time.Millisecond):
	}

	store.flushMu.Unlock()

	select {
	case err := <-errCh:
		t.Fatalf("QueryAggregatedHistory() error = %v", err)
	case buckets := <-resultCh:
		if len(buckets) != 1 {
			t.Fatalf("len(buckets) = %d, want 1", len(buckets))
		}
		if buckets[0].TotalRequests != 1 {
			t.Fatalf("TotalRequests = %d, want 1", buckets[0].TotalRequests)
		}
		if buckets[0].SuccessCount != 1 {
			t.Fatalf("SuccessCount = %d, want 1", buckets[0].SuccessCount)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("QueryAggregatedHistory() did not finish after flushMu released")
	}

	store.flushMu.Lock()
}
