package main

import (
	"math"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
)

func (s *relayServer) admitDirectProviderAccount(c *gin.Context, accountID string, gateway *providerGatewaySpec, model string) bool {
	if s.manifest == nil || s.manifest.MaxAccountConcurrency <= 0 {
		return true
	}
	accountID = strings.TrimSpace(accountID)
	if accountID == "" {
		writeAPIError(c, http.StatusBadGateway, "provider gateway requires one bound account for concurrency tracking", "provider_account_not_available")
		return false
	}
	if s.policy == nil || s.policy.tracker == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "account concurrency tracking is not initialized", "account_concurrency_unavailable")
		return false
	}
	tracker := s.policy.tracker
	requestContext := c.Request.Context()
	requestID := accountConcurrencyRequestID(requestContext)
	if requestID == "" {
		writeAPIError(c, http.StatusServiceUnavailable, "account concurrency request identity is unavailable", "account_concurrency_unavailable")
		return false
	}
	selector := &accountConcurrencySelector{manifest: s.manifest, tracker: tracker, locale: s.manifest.Locale}
	startedAt := time.Now()
	reject := func() bool {
		if remaining := tracker.directProviderBackoffRemaining(accountID, gateway, model); remaining > 0 {
			c.Writer.Header().Set("Retry-After", strconv.FormatInt(int64(math.Ceil(remaining.Seconds())), 10))
			writeAPIError(c, http.StatusTooManyRequests, "Provider Retry-After remains active; retry after the advertised interval.", "provider_retry_after_active")
			return false
		}
		failure := selector.concurrencyExceededError(s.manifest.accountByID[accountID], time.Since(startedAt))
		c.Writer.Header().Set("Retry-After", "1")
		writeAPIError(c, failure.HTTPStatus, failure.Message, failure.Code)
		return false
	}
	if requestContext.Err() != nil {
		return false
	}
	if tracker.tryReserveDirectProviderSlot(requestID, accountID, gateway, model, selector.maxConcurrency()) {
		return requestContext.Err() == nil
	}
	wait := selector.waitDuration()
	if wait <= 0 || !tracker.tryBeginAccountWait(defaultAccountConcurrencyMaxWaiting) {
		return reject()
	}
	defer tracker.endAccountWait()
	deadline := startedAt.Add(wait)
	for {
		changed := tracker.accountConcurrencyChangeSignal()
		if requestContext.Err() != nil {
			return false
		}
		if !time.Now().Before(deadline) {
			return reject()
		}
		if tracker.tryReserveDirectProviderSlot(requestID, accountID, gateway, model, selector.maxConcurrency()) {
			return requestContext.Err() == nil
		}
		if waitForAccountConcurrencyChange(requestContext, changed, deadline) != nil {
			return false
		}
	}
}

type providerBackoffKey struct {
	accountID string
	baseURL   string
	model     string
}

func directProviderBackoffKey(accountID string, gateway *providerGatewaySpec, model string) providerBackoffKey {
	return providerBackoffKey{
		accountID: strings.TrimSpace(accountID),
		baseURL:   strings.TrimRight(strings.TrimSpace(gateway.BaseURL), "/"),
		model:     strings.ToLower(strings.TrimSpace(model)),
	}
}

func (t *requestUsageTracker) tryReserveDirectProviderSlot(requestID, accountID string, gateway *providerGatewaySpec, model string, maxConcurrent int) bool {
	key := directProviderBackoffKey(accountID, gateway, model)
	t.mu.Lock()
	defer t.mu.Unlock()
	if time.Now().Before(t.providerBackoffs[key]) {
		return false
	}
	delete(t.providerBackoffs, key)
	return t.tryReserveAccountSlotLocked(requestID, "cockpit-provider:"+key.accountID, maxConcurrent)
}

func (t *requestUsageTracker) directProviderBackoffRemaining(accountID string, gateway *providerGatewaySpec, model string) time.Duration {
	key := directProviderBackoffKey(accountID, gateway, model)
	t.mu.Lock()
	defer t.mu.Unlock()
	remaining := time.Until(t.providerBackoffs[key])
	if remaining <= 0 {
		delete(t.providerBackoffs, key)
		return 0
	}
	return remaining
}

func directProviderRetryAfter(value string, now time.Time) time.Duration {
	value = strings.TrimSpace(value)
	if value == "" {
		return 0
	}
	digits := true
	for _, character := range value {
		if character < '0' || character > '9' {
			digits = false
			break
		}
	}
	if digits {
		seconds, err := strconv.ParseUint(value, 10, 64)
		if err != nil || seconds > uint64(math.MaxInt64/int64(time.Second)) {
			return time.Duration(math.MaxInt64)
		}
		return time.Duration(seconds) * time.Second
	}
	deadline, err := http.ParseTime(value)
	if err != nil || !deadline.After(now) {
		return 0
	}
	return deadline.Sub(now)
}

func (t *requestUsageTracker) recordDirectProviderBackoff(accountID string, gateway *providerGatewaySpec, model string, status int, retryAfter string, now time.Time) {
	if status != http.StatusTooManyRequests && status != http.StatusServiceUnavailable {
		return
	}
	delay := directProviderRetryAfter(retryAfter, now)
	if delay <= 0 {
		return
	}
	key := directProviderBackoffKey(accountID, gateway, model)
	deadline := now.Add(delay)
	t.mu.Lock()
	defer t.mu.Unlock()
	for existingKey, existingDeadline := range t.providerBackoffs {
		if !existingDeadline.After(now) {
			delete(t.providerBackoffs, existingKey)
		}
	}
	if !deadline.After(t.providerBackoffs[key]) {
		return
	}
	if t.providerBackoffs == nil {
		t.providerBackoffs = make(map[providerBackoffKey]time.Time)
	}
	t.providerBackoffs[key] = deadline
	t.notifyAccountConcurrencyChangeLocked()
}
