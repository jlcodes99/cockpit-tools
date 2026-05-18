package metrics

import (
	"path/filepath"
	"testing"
	"time"
)

func TestFlushBufferLocked_RequeuesRecordsOnInsertFailure(t *testing.T) {
	store, err := NewSQLiteStore(&SQLiteStoreConfig{
		DBPath:        filepath.Join(t.TempDir(), "metrics.db"),
		RetentionDays: 7,
	})
	if err != nil {
		t.Fatalf("NewSQLiteStore() err = %v", err)
	}
	defer func() {
		_ = store.Close()
	}()

	store.writeBuffer = append(store.writeBuffer, PersistentRecord{
		MetricsKey: "k1",
		BaseURL:    "https://example.com",
		KeyMask:    "sk-***",
		Timestamp:  time.Now(),
		APIType:    "messages",
	})

	if err := store.db.Close(); err != nil {
		t.Fatalf("close db err = %v", err)
	}

	store.flushBufferLocked()

	if len(store.writeBuffer) != 1 {
		t.Fatalf("writeBuffer len = %d, want 1", len(store.writeBuffer))
	}
	if store.writeBuffer[0].MetricsKey != "k1" {
		t.Fatalf("writeBuffer[0].MetricsKey = %s, want k1", store.writeBuffer[0].MetricsKey)
	}
}
