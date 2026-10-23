package main

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func quotaProtectionFixture(now time.Time, remaining int) (*accountSpec, *quotaReserveSnapshot) {
	cutoff, present, updated, verified := 13, true, now.Unix(), now.UnixMilli()
	account := &accountSpec{ID: "protected", AuthID: "protected.json", QuotaReserve: &quotaReserveSpec{
		HourlyThresholdPercent: &cutoff, WeeklyThresholdPercent: &cutoff,
	}}
	snapshot := &quotaReserveSnapshot{
		SnapshotUpdatedAtUnixSeconds:             &updated,
		SnapshotVerifiedQueryStartedAtUnixMillis: &verified,
		HourlyRemainingPercent:                   &remaining, WeeklyRemainingPercent: &remaining,
		HourlyWindowPresent: &present, WeeklyWindowPresent: &present,
	}
	return account, snapshot
}

func TestQuotaProtectionRequiresLiveEvidenceAndRespectsTheReserve(t *testing.T) {
	now := time.Now()
	account, snapshot := quotaProtectionFixture(now, 80)
	q := &quotaProtectionCoordinator{}
	snapshot.SnapshotVerifiedQueryStartedAtUnixMillis = nil
	if _, err := q.acquire(account, snapshot, "cold", now); err == nil {
		t.Fatal("persisted display quota must not bypass live verification")
	}
	started := now.UnixMilli()
	snapshot.SnapshotVerifiedQueryStartedAtUnixMillis = &started
	if _, err := q.acquire(account, snapshot, "", now); err == nil {
		t.Fatal("missing request lifecycle identity must fail closed")
	}
	if _, err := q.acquire(account, snapshot, "live", now); err != nil {
		t.Fatal(err)
	}
	*snapshot.HourlyRemainingPercent = 13
	if _, err := q.acquire(account, snapshot, "blocked", now); err == nil {
		t.Fatal("reserve plus safety margin must block at its cutoff")
	}
	if _, err := q.acquire(&accountSpec{ID: "unrelated"}, nil, "", now); err != nil {
		t.Fatalf("unconfigured/API-key account was affected: %v", err)
	}
}

func TestQuotaProtectionCriticalAdmissionIsAtomicAcrossSelectors(t *testing.T) {
	now := time.Now()
	account, snapshot := quotaProtectionFixture(now, 18)
	q := &quotaProtectionCoordinator{}
	var admitted atomic.Int32
	var workers sync.WaitGroup
	start := make(chan struct{})
	for i := 0; i < 32; i++ {
		workers.Add(1)
		go func(i int) {
			defer workers.Done()
			<-start
			if _, err := q.acquire(account, snapshot, string(rune('A'+i)), now); err == nil {
				admitted.Add(1)
			}
		}(i)
	}
	close(start)
	workers.Wait()
	if admitted.Load() != 1 {
		t.Fatalf("critical zone admitted %d concurrent requests", admitted.Load())
	}
}

func TestQuotaProtectionCompletionWaitsForAQueryStartedAfterSettlement(t *testing.T) {
	now := time.Now()
	account, snapshot := quotaProtectionFixture(now, 18)
	q := &quotaProtectionCoordinator{}
	if _, err := q.acquire(account, snapshot, "first", now); err != nil {
		t.Fatal(err)
	}
	if err := q.beforeSend(account, snapshot, "first", now); err != nil {
		t.Fatal(err)
	}
	q.release("first", now)
	if _, err := q.acquire(account, snapshot, "second", now.Add(2*time.Second)); err == nil {
		t.Fatal("old query must not restore allowance after completion")
	}
	// A response completing later is still insufficient when its query began
	// during the previous request; use the query-start watermark, not file time.
	oldStart := now.Add(500 * time.Millisecond).UnixMilli()
	snapshot.SnapshotVerifiedQueryStartedAtUnixMillis = &oldStart
	if _, err := q.acquire(account, snapshot, "second", now.Add(2*time.Second)); err == nil {
		t.Fatal("query started before settlement was accepted")
	}
	laterStart := now.Add(time.Second).UnixMilli()
	snapshot.SnapshotVerifiedQueryStartedAtUnixMillis = &laterStart
	if _, err := q.acquire(account, snapshot, "second", now.Add(2*time.Second)); err != nil {
		t.Fatal(err)
	}
}

func TestQuotaProtectionRequestCleanupReleasesWithoutConfiguredConcurrency(t *testing.T) {
	now := time.Now()
	account, snapshot := quotaProtectionFixture(now, 18)
	q := &quotaProtectionCoordinator{}
	tracker := newRequestUsageTracker()
	tracker.quotaAdmission = q
	if _, err := q.acquire(account, snapshot, "finished", now); err != nil {
		t.Fatal(err)
	}
	if err := q.beforeSend(account, snapshot, "finished", now); err != nil {
		t.Fatal(err)
	}
	tracker.releaseAccountSlots("finished")
	q.mu.Lock()
	state := q.accounts[account.ID]
	if len(state.requests) != 0 || state.completedAt.IsZero() {
		t.Fatal("request completion leaked admission or missed settlement fence")
	}
	q.mu.Unlock()
}

type quotaChangingSelector struct {
	inner  coreauth.Selector
	update func()
}

func (s *quotaChangingSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	auth, err := s.inner.Pick(ctx, provider, model, opts, auths)
	s.update()
	return auth, err
}

func TestQuotaProtectionRechecksAfterTheInnerSelectionAndUsesOtherAccounts(t *testing.T) {
	now := time.Now()
	account, snapshot := quotaProtectionFixture(now, 80)
	other := &accountSpec{ID: "other", AuthID: "other.json"}
	m := &manifest{
		Accounts:        []accountSpec{*account, *other},
		accountByID:     map[string]*accountSpec{account.ID: account, other.ID: other},
		accountByAuthID: map[string]*accountSpec{account.AuthID: account, other.AuthID: other},
	}
	store := newQuotaReserveStateStore("", m)
	store.snapshot.Store(map[string]quotaReserveSnapshot{account.ID: *snapshot})
	selector := &quotaProtectionSelector{manifest: m, quota: store, admission: store.protectionCoordinator(), fallback: &quotaChangingSelector{
		inner: &orderedAuthSelector{order: []string{account.AuthID, other.AuthID}},
		update: func() {
			blocked := 13
			replacement := *snapshot
			replacement.HourlyRemainingPercent = &blocked
			store.snapshot.Store(map[string]quotaReserveSnapshot{account.ID: replacement})
		},
	}}
	auths := []*coreauth.Auth{testAuth(account.AuthID), testAuth(other.AuthID)}
	selected, err := selector.Pick(accountConcurrencyRequestContext("after-wait"), "codex", "gpt-5.4", cliproxyexecutor.Options{}, auths)
	if err != nil || selected == nil || selected.ID != other.AuthID {
		t.Fatalf("latest snapshot was not checked after selection: auth=%v error=%v", selected, err)
	}
	_, err = selector.Pick(accountConcurrencyRequestContext("only-protected"), "codex", "gpt-5.4", cliproxyexecutor.Options{}, auths[:1])
	var apiError *coreauth.Error
	if !errors.As(err, &apiError) || apiError.Code != "quota_reserved" {
		t.Fatalf("expected typed quota rejection, got %v", err)
	}
}

func TestQuotaProtectionPreparationCancellationAndFinalSendFence(t *testing.T) {
	now := time.Now()
	account, snapshot := quotaProtectionFixture(now, 80)
	q := &quotaProtectionCoordinator{}
	for _, id := range []string{"first", "prepared"} {
		if _, err := q.acquire(account, snapshot, id, now); err != nil {
			t.Fatal(err)
		}
	}
	remaining := 18
	snapshot.HourlyRemainingPercent = &remaining
	if err := q.beforeSend(account, snapshot, "first", now); err == nil {
		t.Fatal("already prepared parallel requests bypassed the critical fence")
	}
	q.release("prepared", now)
	if !q.accounts[account.ID].completedAt.IsZero() {
		t.Fatal("preparation cancellation invented pending consumption")
	}
	if err := q.beforeSend(account, snapshot, "first", now); err != nil {
		t.Fatal(err)
	}
	q.release("first", now)
	if q.accounts[account.ID].completedAt.IsZero() {
		t.Fatal("dispatch completion missed the settlement fence")
	}
}

func TestQuotaProtectionErrorUsesHostTranslations(t *testing.T) {
	m := &manifest{Locale: "zh-CN", GatewayErrorMessages: map[string]map[string]string{
		"quota_reserved": {"zh-cn": "quota-localized"},
	}}
	err := localizeQuotaProtectionError(m, quotaProtectionError("internal reason"))
	var typed *coreauth.Error
	if !errors.As(err, &typed) || typed.Code != "quota_reserved" || typed.Message != "quota-localized" {
		t.Fatalf("localized error lost its machine contract: %v", err)
	}
}

func TestQuotaProtectionMiddlewareInjectsTheSendFenceAndReleasesAdmission(t *testing.T) {
	gin.SetMode(gin.TestMode)
	now := time.Now()
	account, snapshot := quotaProtectionFixture(now, 18)
	m := &manifest{Accounts: []accountSpec{*account}, accountByID: map[string]*accountSpec{account.ID: account}, accountByAuthID: map[string]*accountSpec{account.AuthID: account}}
	store := newQuotaReserveStateStore("", m)
	store.snapshot.Store(map[string]quotaReserveSnapshot{account.ID: *snapshot})
	tracker := newRequestUsageTracker()
	selector := buildCoreAuthSelectorWithConcurrency(nil, &orderedAuthSelector{order: []string{account.AuthID}}, m, store, tracker)
	policy := &requestPolicy{manifest: m, tracker: tracker}
	router := gin.New()
	router.Use(policy.middleware())
	router.POST("/v1/responses", func(c *gin.Context) {
		ctx := c.Request.Context()
		selected, err := selector.Pick(ctx, "codex", "gpt-5.4", cliproxyexecutor.Options{}, []*coreauth.Auth{testAuth(account.AuthID)})
		if err != nil || selected == nil {
			t.Errorf("admission failed: %v", err)
			c.Status(500)
			return
		}
		if err := cliproxyexecutor.CheckUpstreamSend(ctx, selected.ID); err != nil {
			t.Errorf("send fence rejected valid admission: %v", err)
			c.Status(500)
			return
		}
		c.Status(http.StatusOK)
	})
	response := httptest.NewRecorder()
	router.ServeHTTP(response, httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader("{}")))
	if response.Code != http.StatusOK {
		t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
	}
	q := store.protectionCoordinator()
	q.mu.Lock()
	defer q.mu.Unlock()
	state := q.accounts[account.ID]
	if state == nil || len(state.requests) != 0 || state.completedAt.IsZero() {
		t.Fatal("request lifecycle did not enforce and release the send fence")
	}
}

func TestQuotaProtectionWebSocketTurnsWaitForSettledSnapshots(t *testing.T) {
	now := time.Now()
	account, snapshot := quotaProtectionFixture(now, 18)
	m := &manifest{Accounts: []accountSpec{*account}, accountByID: map[string]*accountSpec{account.ID: account}, accountByAuthID: map[string]*accountSpec{account.AuthID: account}}
	store := newQuotaReserveStateStore("", m)
	store.snapshot.Store(map[string]quotaReserveSnapshot{account.ID: *snapshot})
	tracker := newRequestUsageTracker()
	hook := &authHook{manifest: m, tracker: tracker}
	selector := buildCoreAuthSelectorWithConcurrency(nil, &orderedAuthSelector{order: []string{account.AuthID}}, m, store, tracker)
	// A WebSocket keeps the same request context and ID for several responses.
	ctx := cliproxyexecutor.WithUpstreamSendGuard(accountConcurrencyRequestContext("persistent-ws"), tracker.quotaProtectionBeforeSend)
	auths := []*coreauth.Auth{testAuth(account.AuthID)}
	turn := func() error {
		selected, err := selector.Pick(ctx, "codex", "gpt-5.4", cliproxyexecutor.Options{}, auths)
		if err != nil {
			return err
		}
		if err := cliproxyexecutor.CheckUpstreamSend(ctx, selected.ID); err != nil {
			return err
		}
		hook.OnResult(ctx, coreauth.Result{AuthID: selected.ID, Success: true})
		return nil
	}
	if err := turn(); err != nil {
		t.Fatalf("first turn: %v", err)
	}
	if err := turn(); err == nil {
		t.Fatal("next turn reused the old connection permit and bypassed settlement")
	}
	admission := store.protectionCoordinator()
	admission.mu.Lock()
	if len(admission.accounts[account.ID].requests) != 0 {
		t.Fatal("completed WebSocket turn held connection-wide admission")
	}
	admission.accounts[account.ID].completedAt = time.Now().Add(-2 * time.Second)
	admission.mu.Unlock()
	verified, updated := time.Now().UnixMilli(), time.Now().Unix()
	refreshed := *snapshot
	refreshed.SnapshotVerifiedQueryStartedAtUnixMillis = &verified
	refreshed.SnapshotUpdatedAtUnixSeconds = &updated
	store.snapshot.Store(map[string]quotaReserveSnapshot{account.ID: refreshed})
	if err := turn(); err != nil {
		t.Fatalf("new settled snapshot did not restore the next turn: %v", err)
	}
	if err := turn(); err == nil {
		t.Fatal("later turn bypassed its own settlement fence")
	}
}
