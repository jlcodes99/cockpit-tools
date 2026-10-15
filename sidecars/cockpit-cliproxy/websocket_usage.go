package main

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"sync"
	"time"

	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

type websocketUsageKey struct{}

var websocketUsageContextKey websocketUsageKey

// Owned by the request context, so queued callbacks remain valid after the
// socket closes without retaining connection state in the global tracker.
type websocketUsageSink struct {
	mu           sync.Mutex
	connectionID string
	seen         map[string]struct{}
	emit         func(usagePayload)
}

func newWebsocketUsageSink(id string, emit func(usagePayload)) *websocketUsageSink {
	return &websocketUsageSink{connectionID: id, seen: make(map[string]struct{}), emit: emit}
}

func (s *websocketUsageSink) record(record coreusage.Record, payload usagePayload) {
	// The SDK has no response ID. Its per-execution start time has nanosecond
	// precision; account and model distinguish separate upstream attempts.
	id := websocketUsageRequestID(s.connectionID, record.AuthID, record.Model, record.RequestedAt)
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, ok := s.seen[id]; ok {
		return
	}
	s.seen[id] = struct{}{}
	payload.RequestID = id
	s.emit(payload)
}

func websocketUsageRequestID(connectionID, authID, model string, requestedAt time.Time) string {
	identity, _ := json.Marshal([]string{connectionID, authID, model, requestedAt.UTC().Format("2006-01-02T15:04:05.000000000Z")})
	if requestedAt.IsZero() {
		identity = []byte(rand.Text())
	}
	return fmt.Sprintf("%s:execution:%x", connectionID, sha256.Sum256(identity))
}
