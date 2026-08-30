package auth

import "context"

// ResetAuthState is the embedded-host compatibility form of ResetQuota. It
// also drops per-model state entries so older Cockpit account-reset semantics
// remain intact.
func (m *Manager) ResetAuthState(ctx context.Context, authID string) (*Auth, error) {
	updated, _, errReset := m.ResetQuota(ctx, authID)
	if errReset != nil || updated == nil {
		return updated, errReset
	}
	m.mu.Lock()
	if current := m.auths[authID]; current != nil {
		current.ModelStates = nil
		updated = current.Clone()
	}
	m.mu.Unlock()
	if updated != nil {
		_ = m.persist(ctx, updated)
		if m.scheduler != nil {
			m.scheduler.upsertAuth(updated)
		}
	}
	return updated, nil
}
