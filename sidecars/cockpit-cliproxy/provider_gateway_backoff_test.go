package main

import (
	"context"
	"fmt"
	"math"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
)

func TestDirectProviderBackoffWaitsAcrossKeysWithoutBlockingOtherModels(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, routeKind := range []string{"direct", "fixed"} {
		t.Run(routeKind, func(t *testing.T) {
			var hits atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				writer.Header().Set("Content-Type", "application/json")
				if hits.Add(1) == 1 {
					writer.Header().Set("Retry-After", "1")
					writer.WriteHeader(http.StatusTooManyRequests)
					fmt.Fprint(writer, `{"error":{"code":"rate_limit_exceeded","message":"capacity"}}`)
					return
				}
				fmt.Fprint(writer, `{"id":"resp_test","status":"completed","output":[]}`)
			}))
			defer upstream.Close()
			router, tracker := directConcurrencyRouter(upstream.URL, routeKind, 2500)
			first := httptest.NewRecorder()
			router.ServeHTTP(first, directConcurrencyRequest(routeKind, "first-key", "model-one", false))
			if first.Code != http.StatusTooManyRequests || first.Header().Get("Retry-After") != "1" || hits.Load() != 1 {
				t.Fatalf("original rejection changed: status=%d hits=%d", first.Code, hits.Load())
			}
			notBefore := time.Now().Add(850 * time.Millisecond)
			otherModel := httptest.NewRecorder()
			router.ServeHTTP(otherModel, directConcurrencyRequest(routeKind, "second-key", "model-two", false))
			if otherModel.Code != http.StatusOK || hits.Load() != 2 {
				t.Fatalf("unrelated model blocked: status=%d hits=%d", otherModel.Code, hits.Load())
			}
			second := httptest.NewRecorder()
			router.ServeHTTP(second, directConcurrencyRequest(routeKind, "second-key", "model-one", false))
			if time.Now().Before(notBefore) {
				t.Fatal("another API key bypassed the advertised model backoff")
			}
			if second.Code != http.StatusOK || hits.Load() != 3 {
				t.Fatalf("waited request failed or retried: status=%d hits=%d", second.Code, hits.Load())
			}
			assertDirectConcurrencyReleased(t, tracker)
		})
	}
}

func TestDirectProviderBackoffCancellationAndTimeoutDoNotReachUpstream(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, outcome := range []string{"cancel", "timeout", "no_wait"} {
		t.Run(outcome, func(t *testing.T) {
			var hits atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				hits.Add(1)
				writer.Header().Set("Retry-After", "23")
				writer.WriteHeader(http.StatusServiceUnavailable)
				fmt.Fprint(writer, `{"error":{"code":"server_is_overloaded"}}`)
			}))
			defer upstream.Close()
			waitMS := 80
			if outcome == "cancel" {
				waitMS = 1000
			} else if outcome == "no_wait" {
				waitMS = 0
			}
			router, tracker := directConcurrencyRouter(upstream.URL, "direct", waitMS)
			first := httptest.NewRecorder()
			router.ServeHTTP(first, directConcurrencyRequest("direct", "first-key", "model-one", false))
			if first.Code != http.StatusServiceUnavailable || hits.Load() != 1 {
				t.Fatal("initial upstream rejection was not preserved")
			}
			requestContext, cancel := context.WithCancel(context.Background())
			defer cancel()
			if outcome == "cancel" {
				timer := time.AfterFunc(30*time.Millisecond, cancel)
				defer timer.Stop()
			}
			second := httptest.NewRecorder()
			router.ServeHTTP(second, directConcurrencyRequest("direct", "second-key", "model-one", false).WithContext(requestContext))
			if hits.Load() != 1 {
				t.Fatalf("backoff bypassed: outcome=%s upstream_hits=%d", outcome, hits.Load())
			}
			if outcome != "cancel" && (second.Code != http.StatusTooManyRequests || second.Header().Get("Retry-After") != "23" || !strings.Contains(second.Body.String(), "provider_retry_after_active")) {
				t.Fatalf("remaining backoff lost: status=%d retry_after=%s body=%s", second.Code, second.Header().Get("Retry-After"), second.Body.String())
			}
			assertDirectConcurrencyReleased(t, tracker)
		})
	}
}

func TestDirectProviderRetryAfterParsing(t *testing.T) {
	now := time.Date(2026, 10, 1, 10, 0, 0, 0, time.UTC)
	for _, sample := range []struct {
		name  string
		value string
		want  time.Duration
	}{
		{name: "seconds", value: "23", want: 23 * time.Second},
		{name: "whitespace", value: " 23 ", want: 23 * time.Second},
		{name: "http_date", value: now.Add(40 * time.Second).Format(http.TimeFormat), want: 40 * time.Second},
		{name: "past_date", value: now.Add(-time.Second).Format(http.TimeFormat)},
		{name: "zero", value: "0"},
		{name: "negative", value: "-1"},
		{name: "fractional", value: "0.5"},
		{name: "invalid", value: "invalid"},
		{name: "empty", value: ""},
		{name: "overflow", value: "184467440737095516160", want: time.Duration(math.MaxInt64)},
	} {
		t.Run(sample.name, func(t *testing.T) {
			if got := directProviderRetryAfter(sample.value, now); got != sample.want {
				t.Fatalf("duration=%s, want %s", got, sample.want)
			}
		})
	}
}

func TestDirectProviderBackoffKeepsLaterDeadlineAndIsolatesBindings(t *testing.T) {
	tracker := newRequestUsageTracker()
	gateway := &providerGatewaySpec{BaseURL: "https://gateway.invalid/v1/"}
	equivalentGateway := &providerGatewaySpec{BaseURL: "https://gateway.invalid/v1"}
	now := time.Now()
	tracker.recordDirectProviderBackoff("account-one", gateway, "model-one", http.StatusServiceUnavailable, "23", now)
	tracker.recordDirectProviderBackoff("account-one", gateway, "model-one", http.StatusTooManyRequests, "1", now)
	tracker.recordDirectProviderBackoff("account-one", gateway, "model-one", http.StatusOK, "100", now)
	tracker.recordDirectProviderBackoff("account-one", gateway, "model-one", http.StatusTooManyRequests, "invalid", now)
	key := directProviderBackoffKey("account-one", gateway, "model-one")
	if !tracker.providerBackoffs[key].Equal(now.Add(23 * time.Second)) {
		t.Fatal("shorter, successful, or invalid response changed the active deadline")
	}
	if tracker.tryReserveDirectProviderSlot("blocked", "account-one", equivalentGateway, "MODEL-ONE", 1) {
		t.Fatal("equivalent gateway or model case bypassed the active backoff")
	}
	for _, sample := range []struct {
		name      string
		accountID string
		gateway   *providerGatewaySpec
		model     string
	}{
		{name: "other_model", accountID: "account-one", gateway: gateway, model: "model-two"},
		{name: "other_account", accountID: "account-two", gateway: gateway, model: "model-one"},
		{name: "other_gateway", accountID: "account-one", gateway: &providerGatewaySpec{BaseURL: "https://other.invalid/v1"}, model: "model-one"},
	} {
		if !tracker.tryReserveDirectProviderSlot(sample.name, sample.accountID, sample.gateway, sample.model, 1) {
			t.Fatalf("unrelated binding blocked: %s", sample.name)
		}
		tracker.releaseAccountSlots(sample.name)
	}
	tracker.recordDirectProviderBackoff("account-one", gateway, "model-one", http.StatusTooManyRequests, "40", now)
	if !tracker.providerBackoffs[key].Equal(now.Add(40 * time.Second)) {
		t.Fatal("newer longer backoff was not retained")
	}
	tracker.providerBackoffs[key] = now.Add(-time.Second)
	if !tracker.tryReserveDirectProviderSlot("recovered", "account-one", gateway, "model-one", 1) {
		t.Fatal("expired backoff did not recover")
	}
	tracker.releaseAccountSlots("recovered")
	if len(tracker.providerBackoffs) != 0 {
		t.Fatal("expired backoff was not removed")
	}
	assertDirectConcurrencyReleased(t, tracker)
}

func TestDirectProviderBackoffWaitDoesNotHoldAccountSlot(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var hits atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "application/json")
		if hits.Add(1) == 1 {
			writer.Header().Set("Retry-After", "23")
			writer.WriteHeader(http.StatusTooManyRequests)
			fmt.Fprint(writer, `{"error":{"code":"rate_limit_exceeded"}}`)
			return
		}
		fmt.Fprint(writer, `{"id":"resp_test","status":"completed","output":[]}`)
	}))
	defer upstream.Close()
	router, tracker := directConcurrencyRouter(upstream.URL, "direct", 1000)
	first := httptest.NewRecorder()
	router.ServeHTTP(first, directConcurrencyRequest("direct", "first-key", "model-one", false))
	if first.Code != http.StatusTooManyRequests {
		t.Fatal("initial rejection missing")
	}
	requestContext, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan struct{})
	go func() {
		defer close(done)
		router.ServeHTTP(httptest.NewRecorder(), directConcurrencyRequest("direct", "second-key", "model-one", false).WithContext(requestContext))
	}()
	deadline := time.Now().Add(500 * time.Millisecond)
	for {
		tracker.mu.Lock()
		waiters := tracker.accountWaiters
		occupied := len(tracker.accountInFlight)
		tracker.mu.Unlock()
		if occupied != 0 {
			t.Fatal("cooling request held an account slot")
		}
		if waiters == 1 {
			break
		}
		if !time.Now().Before(deadline) {
			t.Fatal("backoff waiter was not registered")
		}
		time.Sleep(time.Millisecond)
	}
	other := httptest.NewRecorder()
	router.ServeHTTP(other, directConcurrencyRequest("direct", "first-key", "model-two", false))
	if other.Code != http.StatusOK || hits.Load() != 2 {
		t.Fatalf("cooling waiter blocked another model: status=%d hits=%d", other.Code, hits.Load())
	}
	cancel()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("cancelled backoff waiter did not finish")
	}
	assertDirectConcurrencyReleased(t, tracker)
}
