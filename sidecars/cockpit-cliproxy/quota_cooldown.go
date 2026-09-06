package main

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// The host sends only confirmed quota observations. This state is independent
// of retryable auth/model errors, so ResetAuthState cannot remove the guard.
type quotaCooldownState struct {
	Exhausted   bool   `json:"exhausted"`
	ResetAtMS   *int64 `json:"resetAtMs"`
	UpdatedAtMS int64  `json:"updatedAtMs"`
}

func (s quotaCooldownState) active(now time.Time) bool {
	return s.Exhausted && (s.ResetAtMS == nil || *s.ResetAtMS > now.UnixMilli())
}

type quotaCooldownStateStore struct {
	path     string
	snapshot atomic.Value
	mu       sync.Mutex
	lastHash [sha256.Size]byte
	hasHash  bool
}

func newQuotaCooldownStateStore(path string, m *manifest) *quotaCooldownStateStore {
	s := &quotaCooldownStateStore{path: strings.TrimSpace(path)}
	initial := make(map[string]quotaCooldownState)
	if m != nil {
		for _, account := range m.Accounts {
			if account.QuotaCooldown != nil {
				initial[account.ID] = *account.QuotaCooldown
			} else if account.RemainingQuota != nil {
				initial[account.ID] = quotaCooldownState{Exhausted: *account.RemainingQuota == 0}
			}
		}
	}
	s.snapshot.Store(initial)
	return s
}

func (s *quotaCooldownStateStore) load() error {
	if s == nil || s.path == "" {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	content, err := os.ReadFile(s.path)
	if err != nil {
		return err
	}
	hash := sha256.Sum256(content)
	if s.hasHash && hash == s.lastHash {
		return nil
	}
	var state quotaPoolStateFile
	if err := json.Unmarshal(content, &state); err != nil {
		return err
	}
	next := make(map[string]quotaCooldownState)
	for id, account := range state.Accounts {
		cooldown := account.Cooldown
		if cooldown == nil {
			cooldown = legacyQuotaCooldownFromPoolState(account)
		}
		if cooldown != nil {
			next[id] = *cooldown
		}
	}
	if previous, ok := s.snapshot.Load().(map[string]quotaCooldownState); ok {
		for id, current := range previous {
			if incoming, exists := next[id]; exists && current.UpdatedAtMS > incoming.UpdatedAtMS {
				next[id] = current
			}
		}
	}
	s.snapshot.Store(next)
	s.lastHash, s.hasHash = hash, true
	return nil
}

func legacyQuotaCooldownFromPoolState(account quotaPoolAccountState) *quotaCooldownState {
	windows := []*quotaPoolWindowState{account.Primary, account.Secondary}
	exhausted := false
	var resetAtMS *int64
	for _, window := range windows {
		if !quotaWindowPresent(window) || window.RemainingPercent == nil || *window.RemainingPercent != 0 {
			continue
		}
		if window.ResetAt != nil && *window.ResetAt > 0 && *window.ResetAt*1000 <= time.Now().UnixMilli() {
			continue
		}
		exhausted = true
		if window.ResetAt == nil || *window.ResetAt <= 0 {
			resetAtMS = nil
		} else if resetAtMS == nil || *resetAtMS < *window.ResetAt*1000 {
			value := *window.ResetAt * 1000
			resetAtMS = &value
		}
	}
	if !exhausted {
		return &quotaCooldownState{}
	}
	return &quotaCooldownState{Exhausted: true, ResetAtMS: resetAtMS, UpdatedAtMS: 0}
}

func (s *quotaCooldownStateStore) start(ctx context.Context, emitter *eventEmitter) {
	if s == nil || s.path == "" {
		return
	}
	go func() {
		ticker := time.NewTicker(time.Second)
		defer ticker.Stop()
		lastError := ""
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				if err := s.load(); err != nil {
					// Keep the last good snapshot on partial writes/read failures.
					if err.Error() != lastError && emitter != nil {
						emitter.emit(map[string]any{"type": "quota_cooldown_state_error", "message": err.Error()})
					}
					lastError = err.Error()
				} else {
					lastError = ""
				}
			}
		}
	}()
}

func accountQuotaExhausted(m *manifest, account *accountSpec, now time.Time) bool {
	if account == nil || strings.EqualFold(strings.TrimSpace(account.AuthKind), "api_key") {
		return false
	}
	if m != nil && m.quotaCooldowns != nil {
		if snapshot, ok := m.quotaCooldowns.snapshot.Load().(map[string]quotaCooldownState); ok {
			return snapshot[account.ID].active(now)
		}
	}
	if account.QuotaCooldown != nil {
		return account.QuotaCooldown.active(now)
	}
	return account.RemainingQuota != nil && *account.RemainingQuota == 0
}

func authHasQuotaCooldown(auth *coreauth.Auth, now time.Time) bool {
	if auth == nil {
		return false
	}
	if auth.Quota.Exceeded && runtimeAvailabilityBlocked(auth.Unavailable, true, auth.NextRetryAfter, auth.Quota.NextRecoverAt, now) {
		return true
	}
	for _, state := range auth.ModelStates {
		if state != nil && state.Quota.Exceeded && runtimeAvailabilityBlocked(state.Unavailable, true, state.NextRetryAfter, state.Quota.NextRecoverAt, now) {
			return true
		}
	}
	return false
}

// Run before session-affinity/backup selection; a cached binding must not
// bypass a new quota snapshot. The normal selector still enforces reserve
// eligibility, key scopes and model exclusions.
type quotaCooldownSelector struct {
	manifest *manifest
	fallback coreauth.Selector
}

func (s *quotaCooldownSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if isCodexReserveModel(model) {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}
	now := time.Now()
	filtered := make([]*coreauth.Auth, 0, len(auths))
	for _, auth := range auths {
		if !accountQuotaExhausted(s.manifest, accountForAuthInManifest(s.manifest, auth), now) {
			filtered = append(filtered, auth)
		}
	}
	if len(filtered) == 0 && len(auths) > 0 {
		err := noAuthAvailableError(nil)
		return nil, s.ReportAuthSelectionFailure(ctx, provider, model, auths, err)
	}
	return s.fallback.Pick(ctx, provider, model, opts, filtered)
}

func (s *quotaCooldownSelector) Stop() {
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *quotaCooldownSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, auths []*coreauth.Auth, err error) error {
	if reporter, ok := s.fallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, auths, err)
	}
	return err
}
