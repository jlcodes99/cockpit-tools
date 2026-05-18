package metrics

import "testing"

func TestChannelLogStoreRemoveAndShift(t *testing.T) {
	store := NewChannelLogStore()
	store.Record(0, &ChannelLog{Model: "channel-0"})
	store.Record(1, &ChannelLog{Model: "channel-1"})
	store.Record(2, &ChannelLog{Model: "channel-2"})

	store.RemoveAndShift(1)

	channel0Logs := store.Get(0)
	if len(channel0Logs) != 1 || channel0Logs[0].Model != "channel-0" {
		t.Fatalf("channel 0 logs = %#v, want channel-0", channel0Logs)
	}

	shiftedLogs := store.Get(1)
	if len(shiftedLogs) != 1 || shiftedLogs[0].Model != "channel-2" {
		t.Fatalf("shifted logs = %#v, want channel-2", shiftedLogs)
	}

	if got := store.Get(2); got != nil {
		t.Fatalf("channel 2 logs = %#v, want nil", got)
	}
}

func TestChannelLogStoreUpdateReturnsDeletedAfterRemoveAndShift(t *testing.T) {
	store := NewChannelLogStore()
	log := &ChannelLog{RequestID: "req-delete", Status: StatusPending}
	store.Record(0, log)

	store.RemoveAndShift(0)

	status, actualIndex := store.Update(0, "req-delete", func(log *ChannelLog) {
		log.Status = StatusCompleted
	})
	if status != UpdateMissingDeleted {
		t.Fatalf("status = %v, want %v", status, UpdateMissingDeleted)
	}
	if actualIndex != -1 {
		t.Fatalf("actualIndex = %d, want -1", actualIndex)
	}
}

func TestChannelLogStoreUpdateReturnsEvictedWhileStillTracked(t *testing.T) {
	store := NewChannelLogStore()
	store.Record(0, &ChannelLog{RequestID: "req-evicted", Status: StatusPending})

	store.mu.Lock()
	store.logs[0] = []*ChannelLog{}
	store.mu.Unlock()

	status, actualIndex := store.Update(0, "req-evicted", func(log *ChannelLog) {
		log.Status = StatusCompleted
	})
	if status != UpdateMissingEvicted {
		t.Fatalf("status = %v, want %v", status, UpdateMissingEvicted)
	}
	if actualIndex != 0 {
		t.Fatalf("actualIndex = %d, want 0", actualIndex)
	}
}

func TestChannelLogStoreRemoveAndShiftClearsEvictedInFlightRequestAtDeletedIndex(t *testing.T) {
	store := NewChannelLogStore()
	store.Record(1, &ChannelLog{RequestID: "req-stale", Status: StatusPending})

	store.mu.Lock()
	store.logs[1] = []*ChannelLog{}
	store.mu.Unlock()

	store.RemoveAndShift(1)

	status, actualIndex := store.Update(1, "req-stale", func(log *ChannelLog) {
		log.Status = StatusCompleted
	})
	if status != UpdateMissingDeleted {
		t.Fatalf("status = %v, want %v", status, UpdateMissingDeleted)
	}
	if actualIndex != -1 {
		t.Fatalf("actualIndex = %d, want -1", actualIndex)
	}
}

func TestChannelLogStoreUpdateReturnsShiftedActualIndexForEvictedRequest(t *testing.T) {
	store := NewChannelLogStore()
	store.Record(1, &ChannelLog{RequestID: "req-shifted", Status: StatusPending})

	store.mu.Lock()
	store.logs[1] = []*ChannelLog{}
	store.mu.Unlock()

	store.RemoveAndShift(0)

	status, actualIndex := store.Update(1, "req-shifted", func(log *ChannelLog) {
		log.Status = StatusCompleted
	})
	if status != UpdateMissingEvicted {
		t.Fatalf("status = %v, want %v", status, UpdateMissingEvicted)
	}
	if actualIndex != 0 {
		t.Fatalf("actualIndex = %d, want 0", actualIndex)
	}
}
