package main

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

// Quota protection belongs to a configured reserve, regardless of legacy strict
// flags. The store is shared by normal, affinity and automatic selectors.
type quotaProtectionAccount struct {
	requests    map[string]bool
	completedAt time.Time
}

type quotaProtectionCoordinator struct {
	mu       sync.Mutex
	accounts map[string]*quotaProtectionAccount
}

func manifestHasQuotaReserve(m *manifest) bool {
	if m != nil {
		for i := range m.Accounts {
			if m.Accounts[i].QuotaReserve != nil {
				return true
			}
		}
	}
	return false
}

func (s *quotaReserveStateStore) protectionCoordinator() *quotaProtectionCoordinator {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.admission == nil {
		s.admission = &quotaProtectionCoordinator{accounts: make(map[string]*quotaProtectionAccount)}
	}
	return s.admission
}

func quotaProtectionNear(account *accountSpec, snapshot *quotaReserveSnapshot) bool {
	if account == nil || account.QuotaReserve == nil || snapshot == nil {
		return false
	}
	reserve := account.QuotaReserve
	near := func(threshold, remaining *int, present *bool) bool {
		return (present == nil || *present) && threshold != nil && remaining != nil && *remaining <= max(*threshold+10, 20)
	}
	return near(reserve.HourlyThresholdPercent, snapshot.HourlyRemainingPercent, snapshot.HourlyWindowPresent) ||
		near(reserve.WeeklyThresholdPercent, snapshot.WeeklyRemainingPercent, snapshot.WeeklyWindowPresent)
}

func quotaProtectionLiveQuery(snapshot *quotaReserveSnapshot, now time.Time) bool {
	if snapshot == nil || snapshot.SnapshotVerifiedQueryStartedAtUnixMillis == nil {
		return false
	}
	started := *snapshot.SnapshotVerifiedQueryStartedAtUnixMillis
	return started > 0 && started <= now.UnixMilli() && now.UnixMilli()-started <= quotaReserveMaxSnapshotAge.Milliseconds()
}

func quotaProtectionError(message string) *coreauth.Error {
	return &coreauth.Error{Code: "quota_reserved", Message: message, HTTPStatus: http.StatusTooManyRequests, Retryable: false}
}

func localizeQuotaProtectionError(m *manifest, err error) error {
	var apiError *coreauth.Error
	if m != nil && errors.As(err, &apiError) && apiError.Code == "quota_reserved" {
		if message, found := (&cockpitSelector{manifest: m, locale: m.Locale}).localizedPoolErrorMessage(apiError.Code); found {
			apiError.Message = message
		}
	}
	return err
}

// acquire runs after account-concurrency waits. Its decision and reservation are
// atomic, so concurrent selectors cannot both take the last safe request slot.
func (q *quotaProtectionCoordinator) acquire(account *accountSpec, snapshot *quotaReserveSnapshot, requestID string, now time.Time) (bool, error) {
	if account == nil || account.QuotaReserve == nil {
		return false, nil
	}
	if reason := quotaReserveBlockReasonWithSnapshot(account, snapshot, now); reason != "" {
		return false, quotaProtectionError(reason)
	}
	if !quotaProtectionLiveQuery(snapshot, now) {
		return false, quotaProtectionError("OAuth quota is awaiting a live confirmation")
	}
	if strings.TrimSpace(requestID) == "" {
		return false, quotaProtectionError("OAuth quota request identity is unavailable")
	}
	q.mu.Lock()
	defer q.mu.Unlock()
	if q.accounts == nil {
		q.accounts = make(map[string]*quotaProtectionAccount)
	}
	state := q.accounts[account.ID]
	if state == nil {
		// Reserve configuration contains only bound OAuth accounts; this bound
		// also prevents a malformed manifest from growing unbounded state.
		if len(q.accounts) >= 4096 {
			return false, quotaProtectionError("OAuth quota admission capacity reached")
		}
		state = &quotaProtectionAccount{requests: make(map[string]bool)}
		q.accounts[account.ID] = state
	}
	near := quotaProtectionNear(account, snapshot)
	if near && !state.completedAt.IsZero() && *snapshot.SnapshotVerifiedQueryStartedAtUnixMillis < state.completedAt.Add(time.Second).UnixMilli() {
		return false, quotaProtectionError("OAuth quota is awaiting settled consumption")
	}
	if _, held := state.requests[requestID]; held {
		return false, nil
	}
	if near && len(state.requests) > 0 {
		return false, quotaProtectionError("OAuth quota is reserved by an in-flight request")
	}
	state.requests[requestID] = false
	return true, nil
}

func (q *quotaProtectionCoordinator) complete(accountID, requestID string, now time.Time) {
	if q == nil || requestID == "" {
		return
	}
	q.mu.Lock()
	defer q.mu.Unlock()
	if state := q.accounts[accountID]; state != nil {
		if dispatched, held := state.requests[requestID]; held {
			delete(state.requests, requestID)
			if dispatched {
				state.completedAt = now
			}
		}
	}
}

func (t *requestUsageTracker) completeQuotaProtection(ctx context.Context, authID string) {
	t.mu.Lock()
	admission, manifest := t.quotaAdmission, t.quotaManifest
	t.mu.Unlock()
	if admission != nil && manifest != nil {
		if account := manifest.accountByAuthID[strings.ToLower(strings.TrimSpace(authID))]; account != nil {
			admission.complete(account.ID, accountConcurrencyRequestID(ctx), time.Now())
		}
	}
}

func (q *quotaProtectionCoordinator) release(requestID string, now time.Time) {
	if q == nil || strings.TrimSpace(requestID) == "" {
		return
	}
	q.mu.Lock()
	defer q.mu.Unlock()
	for _, state := range q.accounts {
		if dispatched, held := state.requests[requestID]; held {
			delete(state.requests, requestID)
			if dispatched {
				state.completedAt = now
			}
		}
	}
}

// beforeSend is invoked for each HTTP dispatch or WebSocket message, including
// retries after credential refresh and waits for existing connections.
func (q *quotaProtectionCoordinator) beforeSend(account *accountSpec, snapshot *quotaReserveSnapshot, requestID string, now time.Time) error {
	if account == nil || account.QuotaReserve == nil {
		return nil
	}
	if reason := quotaReserveBlockReasonWithSnapshot(account, snapshot, now); reason != "" {
		return quotaProtectionError(reason)
	}
	if !quotaProtectionLiveQuery(snapshot, now) {
		return quotaProtectionError("OAuth quota is awaiting a live confirmation")
	}
	q.mu.Lock()
	defer q.mu.Unlock()
	state := q.accounts[account.ID]
	if state == nil {
		return quotaProtectionError("OAuth quota admission is unavailable")
	}
	if _, held := state.requests[requestID]; !held {
		return quotaProtectionError("OAuth quota request has no admission")
	}
	if quotaProtectionNear(account, snapshot) && (len(state.requests) > 1 ||
		(!state.completedAt.IsZero() && *snapshot.SnapshotVerifiedQueryStartedAtUnixMillis < state.completedAt.Add(time.Second).UnixMilli())) {
		return quotaProtectionError("OAuth quota is reserved or awaiting settled consumption")
	}
	state.requests[requestID] = true
	return nil
}

func (t *requestUsageTracker) quotaProtectionBeforeSend(ctx context.Context, authID string) error {
	t.mu.Lock()
	admission, manifest, store := t.quotaAdmission, t.quotaManifest, t.quotaState
	t.mu.Unlock()
	if admission == nil || manifest == nil || store == nil {
		return nil
	}
	account := manifest.accountByAuthID[strings.ToLower(strings.TrimSpace(authID))]
	if account == nil || account.QuotaReserve == nil {
		return nil
	}
	return localizeQuotaProtectionError(manifest, admission.beforeSend(account, store.forAccount(account.ID), accountConcurrencyRequestID(ctx), time.Now()))
}

func (t *requestUsageTracker) releaseQuotaProtection(requestID string) {
	t.mu.Lock()
	admission := t.quotaAdmission
	t.mu.Unlock()
	admission.release(requestID, time.Now())
}

type quotaProtectionSelector struct {
	manifest  *manifest
	quota     *quotaReserveStateStore
	fallback  coreauth.Selector
	admission *quotaProtectionCoordinator
	tracker   *requestUsageTracker
}

func (s *quotaProtectionSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.fallback == nil || s.admission == nil {
		return nil, fmt.Errorf("quota protection selector is not initialized")
	}
	requestID := accountConcurrencyRequestID(ctx)
	remaining := auths
	excluded := make(map[*coreauth.Auth]struct{})
	var lastError error
	for len(remaining) > 0 {
		auth, err := s.fallback.Pick(ctx, provider, model, opts, remaining)
		if err != nil || auth == nil {
			return auth, err
		}
		account := accountForAuthInManifest(s.manifest, auth)
		var snapshot *quotaReserveSnapshot
		if account != nil {
			snapshot = s.quota.forAccount(account.ID)
		}
		acquired, err := s.admission.acquire(account, snapshot, requestID, time.Now())
		if err == nil {
			if acquired && s.tracker == nil && ctx != nil && ctx.Done() != nil {
				// Direct selector callers without request middleware still release
				// admission on cancellation. Production uses the tracker defer.
				context.AfterFunc(ctx, func() { s.admission.release(requestID, time.Now()) })
			}
			return auth, nil
		}
		lastError = err
		excluded[auth] = struct{}{}
		remaining = excludeAuths(auths, excluded)
	}
	if lastError != nil {
		return nil, localizeQuotaProtectionError(s.manifest, lastError)
	}
	return nil, noAuthAvailableError(nil)
}

func (s *quotaProtectionSelector) OnResult(result coreauth.Result) {
	if s != nil {
		forwardAuthSelectionResult(s.fallback, result)
	}
}

func (s *quotaProtectionSelector) Stop() {
	if s != nil {
		if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
			stoppable.Stop()
		}
	}
}

func (s *quotaProtectionSelector) ReportAuthSelectionFailure(ctx context.Context, provider, model string, candidates []*coreauth.Auth, err error) error {
	if reporter, ok := s.fallback.(coreauth.AuthSelectionFailureReporter); ok {
		return reporter.ReportAuthSelectionFailure(ctx, provider, model, candidates, err)
	}
	return err
}
