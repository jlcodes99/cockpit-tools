package main

import (
	"context"
	"fmt"
	"math"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
)

type providerGatewayAccountContextKey struct{}

type providerGatewayBackoffKey struct {
	account, gateway, model string
}

type providerGatewayBackoffState struct {
	mu        sync.Mutex
	deadlines map[providerGatewayBackoffKey]time.Time
}

const providerGatewayMaxBackoff = 24 * time.Hour
const providerGatewayMaxBackoffEntries = 4096

func parseProviderRetryAfter(value string, now time.Time) time.Duration {
	value = strings.TrimSpace(value)
	if seconds, err := strconv.ParseInt(value, 10, 64); err == nil {
		if seconds <= 0 {
			return 0
		}
		if seconds >= int64(providerGatewayMaxBackoff/time.Second) {
			return providerGatewayMaxBackoff
		}
		return time.Duration(seconds) * time.Second
	}
	deadline, err := http.ParseTime(value)
	if err != nil || !deadline.After(now) {
		return 0
	}
	return min(deadline.Sub(now), providerGatewayMaxBackoff)
}

func (state *providerGatewayBackoffState) remaining(key providerGatewayBackoffKey, now time.Time) time.Duration {
	state.mu.Lock()
	defer state.mu.Unlock()
	deadline := state.deadlines[key]
	if !deadline.After(now) {
		delete(state.deadlines, key)
		return 0
	}
	return deadline.Sub(now)
}

func (state *providerGatewayBackoffState) observe(key providerGatewayBackoffKey, status int, header string, now time.Time) {
	if key.account == "" || (status != http.StatusTooManyRequests && status != http.StatusServiceUnavailable) {
		return
	}
	duration := parseProviderRetryAfter(header, now)
	if duration <= 0 {
		return
	}
	deadline := now.Add(duration)
	state.mu.Lock()
	defer state.mu.Unlock()
	if state.deadlines == nil {
		state.deadlines = make(map[providerGatewayBackoffKey]time.Time)
	}
	if previous := state.deadlines[key]; previous.After(deadline) {
		return
	}
	if len(state.deadlines) >= providerGatewayMaxBackoffEntries {
		for entry, until := range state.deadlines {
			if !until.After(now) {
				delete(state.deadlines, entry)
			}
		}
		if _, exists := state.deadlines[key]; !exists && len(state.deadlines) >= providerGatewayMaxBackoffEntries {
			return
		}
	}
	state.deadlines[key] = deadline
}

func providerGatewayBoundAccount(c *gin.Context) string {
	if c == nil || c.Request == nil {
		return ""
	}
	if account, _ := c.Request.Context().Value(providerGatewayAccountContextKey{}).(string); account != "" {
		return account
	}
	spec, _ := c.Request.Context().Value(clientAPIKeyContextKey).(*apiKeySpec)
	// Automatic selection already owns its admission lifecycle. Do not infer a
	// provider account from the default OAuth pool or from an unbound route.
	if spec == nil || spec.ModelRouting != nil || len(spec.AccountIDs) != 1 {
		return ""
	}
	return strings.TrimSpace(spec.AccountIDs[0])
}

func (s *relayServer) providerGatewayErrorMessage(code string, seconds int) string {
	message := code
	if s.manifest != nil {
		translations := s.manifest.GatewayErrorMessages[code]
		locale := strings.ToLower(strings.TrimSpace(s.manifest.Locale))
		for _, candidate := range []string{locale, strings.Split(locale, "-")[0], "en"} {
			for language, value := range translations {
				if strings.EqualFold(language, candidate) && value != "" {
					message = value
					break
				}
			}
			if message != code {
				break
			}
		}
	}
	return strings.ReplaceAll(message, "{{seconds}}", strconv.Itoa(seconds))
}

func (s *relayServer) rejectProviderBackoff(c *gin.Context, key providerGatewayBackoffKey) bool {
	remaining := s.providerBackoff.remaining(key, time.Now())
	if remaining <= 0 {
		return false
	}
	seconds := int(math.Ceil(remaining.Seconds()))
	c.Header("Retry-After", strconv.Itoa(seconds))
	writeAPIError(c, http.StatusTooManyRequests, s.providerGatewayErrorMessage("provider_retry_after", seconds), "provider_retry_after")
	return true
}

func (s *relayServer) admitProviderGateway(c *gin.Context, key providerGatewayBackoffKey) (func(), bool) {
	noop := func() {}
	if key.account == "" {
		return noop, true
	}
	if s.rejectProviderBackoff(c, key) {
		return noop, false
	}
	if s.manifest == nil || s.manifest.MaxAccountConcurrency <= 0 || s.policy == nil || s.policy.tracker == nil {
		return noop, true
	}
	ctx := relayContext(c)
	tracker := s.policy.tracker
	requestID := accountConcurrencyRequestID(ctx)
	if requestID == "" {
		requestID = fmt.Sprintf("provider-%s", uuid.NewString())
	}
	release := func() { tracker.releaseAccountSlots(requestID) }
	wait := time.Duration(max(s.manifest.AccountConcurrencyWaitMs, 0)) * time.Millisecond
	deadline := time.Now().Add(wait)
	waiting := false
	defer func() {
		if waiting {
			tracker.endAccountWait()
		}
	}()
	for {
		if ctx.Err() != nil {
			release()
			return noop, false
		}
		changed := tracker.accountConcurrencyChangeSignal()
		if tracker.tryReserveAccountSlot(requestID, key.account, s.manifest.MaxAccountConcurrency) {
			// An earlier admitted request may have set backoff while this one waited.
			if s.rejectProviderBackoff(c, key) {
				release()
				return noop, false
			}
			return release, true
		}
		if wait <= 0 || !time.Now().Before(deadline) || (!waiting && !tracker.tryBeginAccountWait(defaultAccountConcurrencyMaxWaiting)) {
			release()
			writeAPIError(c, http.StatusTooManyRequests, s.providerGatewayErrorMessage("account_concurrency_exceeded", 0), "account_concurrency_exceeded")
			return noop, false
		}
		waiting = true
		if err := waitForAccountConcurrencyChange(ctx, changed, deadline); err != nil {
			release()
			return noop, false
		}
	}
}

// The caller's request context is preserved when temporarily binding a fixed
// provider route. This binding never affects other keys or automatic routing.
func bindProviderGatewayAccount(ctx context.Context, account string) context.Context {
	return context.WithValue(ctx, providerGatewayAccountContextKey{}, strings.TrimSpace(account))
}
