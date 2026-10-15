package auth

import (
	"context"
	"strings"
	"time"
)

func (m *Manager) markAuthRecoveryBarrier(authID string) {
	if m == nil {
		return
	}
	authID = strings.TrimSpace(authID)
	if authID == "" {
		return
	}
	m.mu.Lock()
	if m.authRecoveryBarriers == nil {
		m.authRecoveryBarriers = make(map[string]time.Time)
	}
	m.authRecoveryBarriers[authID] = time.Now()
	m.mu.Unlock()
}

// ResetAuthState is the embedded-host compatibility form of ResetQuota. It
// also drops per-model state entries so older Cockpit account-reset semantics
// remain intact.
func (m *Manager) ResetAuthState(ctx context.Context, authID string) (*Auth, error) {
	// Publish a barrier both before and after the reset. The first closes the
	// usual race immediately; the second also covers requests that overlapped
	// the reset itself.
	m.markAuthRecoveryBarrier(authID)
	updated, _, errReset := m.ResetQuota(withSkipPersistWithoutWatermark(ctx), authID)
	if errReset != nil || updated == nil {
		return updated, errReset
	}
	m.mu.Lock()
	if current := m.auths[authID]; current != nil {
		current.ModelStates = nil
		updated = current.Clone()
	}
	m.mu.Unlock()
	m.markAuthRecoveryBarrier(authID)
	if updated != nil {
		if m.scheduler != nil {
			m.scheduler.upsertAuth(updated)
		}
		persisted := updated.Clone()
		go func() {
			_ = m.persist(context.Background(), persisted)
		}()
	}
	return updated, nil
}
