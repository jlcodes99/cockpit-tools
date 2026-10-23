package main

import (
	"bufio"
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func providerAdmissionContext(ctx context.Context, keyID, account string) (*gin.Context, *httptest.ResponseRecorder) {
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	spec := &apiKeySpec{ID: keyID, AccountIDs: []string{account}}
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil).WithContext(context.WithValue(ctx, clientAPIKeyContextKey, spec))
	return c, w
}

func providerAdmissionServer(limit, waitMs int) *relayServer {
	return &relayServer{manifest: &manifest{MaxAccountConcurrency: limit, AccountConcurrencyWaitMs: waitMs}, policy: &requestPolicy{tracker: newRequestUsageTracker()}}
}

func TestProviderRetryAfterParsingAndIsolation(t *testing.T) {
	now := time.Date(2026, 10, 3, 0, 0, 0, 0, time.UTC)
	for _, tc := range []struct {
		value string
		want  time.Duration
	}{
		{"30", 30 * time.Second}, {"0", 0}, {"-1", 0}, {"invalid", 0},
		{"9223372036854775807", providerGatewayMaxBackoff},
		{now.Add(60 * time.Second).Format(http.TimeFormat), time.Minute},
		{now.Add(-time.Second).Format(http.TimeFormat), 0},
	} {
		if got := parseProviderRetryAfter(tc.value, now); got != tc.want {
			t.Fatalf("%q: got %v, want %v", tc.value, got, tc.want)
		}
	}
	var state providerGatewayBackoffState
	key := providerGatewayBackoffKey{"account", "gateway", "model"}
	state.observe(key, 429, "30", now)
	state.observe(key, 503, "2", now)
	if state.remaining(key, now) != 30*time.Second {
		t.Fatal("shorter response replaced existing backoff")
	}
	for _, other := range []providerGatewayBackoffKey{{"other", "gateway", "model"}, {"account", "other", "model"}, {"account", "gateway", "other"}} {
		if state.remaining(other, now) != 0 {
			t.Fatal("backoff leaked to another route")
		}
	}
	if state.remaining(key, now.Add(time.Minute)) != 0 || len(state.deadlines) != 0 {
		t.Fatal("expired backoff not cleared")
	}
	state.observe(key, 500, "30", now)
	state.observe(providerGatewayBackoffKey{"", "gateway", "model"}, 429, "30", now)
	if len(state.deadlines) != 0 {
		t.Fatal("unbound or unsupported status cached")
	}
}

func TestProviderAccountBindingDoesNotInferAutomaticPool(t *testing.T) {
	c, _ := providerAdmissionContext(context.Background(), "key", "account")
	if providerGatewayBoundAccount(c) != "account" {
		t.Fatal("direct binding lost")
	}
	spec := c.Request.Context().Value(clientAPIKeyContextKey).(*apiKeySpec)
	spec.ModelRouting = &modelRoutingSpec{Automatic: true}
	if providerGatewayBoundAccount(c) != "" {
		t.Fatal("automatic pool inferred as provider binding")
	}
	c.Request = c.Request.WithContext(bindProviderGatewayAccount(c.Request.Context(), "fixed"))
	if providerGatewayBoundAccount(c) != "fixed" {
		t.Fatal("fixed route binding lost")
	}
}

func TestProviderBackoffStorageIsBounded(t *testing.T) {
	var state providerGatewayBackoffState
	now := time.Now()
	for i := 0; i < providerGatewayMaxBackoffEntries+100; i++ {
		state.observe(providerGatewayBackoffKey{"account", "gateway", time.Unix(int64(i), 0).String()}, 429, "1", now)
	}
	if len(state.deadlines) != providerGatewayMaxBackoffEntries {
		t.Fatal("backoff cache is unbounded")
	}
	state.observe(providerGatewayBackoffKey{"account", "gateway", "new"}, 429, "10", now.Add(2*time.Second))
	if len(state.deadlines) != 1 {
		t.Fatal("expired entries did not free storage")
	}
}

func TestFixedProviderRouteBindsOwnAccountAndRestoresContext(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"id":"reply"}`)
	}))
	defer upstream.Close()
	gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "test", WireAPI: "responses", UpstreamModel: "test-model", UpstreamModels: []string{"test-model"}}
	spec := &apiKeySpec{AccountIDs: []string{"unrelated-oauth"}, ModelRouting: &modelRoutingSpec{Routes: []modelRouteSpec{{Namespace: "fixed", ProviderAccountID: "provider-account", ProviderGateway: gateway}}}}
	s := providerAdmissionServer(1, 0)
	s.policy.tracker.tryReserveAccountSlot("owner", "provider-account", 1)
	c, w := providerAdmissionContext(context.Background(), "key", "unrelated-oauth")
	original := c.Request
	s.handleExecutorBody(c, spec, []byte(`{"model":"fixed/test-model","input":"hi"}`), sdktranslator.FormatOpenAIResponse, "")
	if w.Code != 429 || calls.Load() != 0 {
		t.Fatal("fixed route bypassed its bound account's limit")
	}
	if c.Request != original || providerGatewayBoundAccount(c) != "unrelated-oauth" {
		t.Fatal("fixed account binding escaped request scope")
	}
	s.policy.tracker.releaseAccountSlots("owner")
	c, w = providerAdmissionContext(context.Background(), "key", "unrelated-oauth")
	s.handleExecutorBody(c, spec, []byte(`{"model":"fixed/test-model","input":"hi"}`), sdktranslator.FormatOpenAIResponse, "")
	if w.Code != 200 || calls.Load() != 1 {
		t.Fatalf("fixed route failed after release: %d %s", w.Code, w.Body.String())
	}
}

func TestProviderAdmissionSharesAccountAcrossKeysAndReleases(t *testing.T) {
	s := providerAdmissionServer(1, 0)
	key := providerGatewayBackoffKey{"account", "gateway", "model"}
	c, _ := providerAdmissionContext(context.Background(), "key-a", "account")
	release, ok := s.admitProviderGateway(c, key)
	if !ok {
		t.Fatal("first request rejected")
	}
	c2, w := providerAdmissionContext(context.Background(), "key-b", "account")
	if _, ok = s.admitProviderGateway(c2, providerGatewayBackoffKey{"account", "gateway", "alias"}); ok || w.Code != 429 {
		t.Fatal("different key/model bypassed account limit")
	}
	release()
	release() // Release is idempotent even when middleware also releases the request.
	if s.policy.tracker.accountInFlightCount("account") != 0 {
		t.Fatal("slot leaked")
	}
	release, ok = s.admitProviderGateway(c2, key)
	if !ok {
		t.Fatal("next request rejected after release")
	}
	release()
	for _, unbound := range []providerGatewayBackoffKey{{"", "gateway", "model"}} {
		r, allowed := s.admitProviderGateway(c, unbound)
		r()
		if !allowed {
			t.Fatal("unbound route changed")
		}
	}
	s.manifest.MaxAccountConcurrency = 0
	r1, ok1 := s.admitProviderGateway(c, key)
	r2, ok2 := s.admitProviderGateway(c2, key)
	r1()
	r2()
	if !ok1 || !ok2 {
		t.Fatal("disabled concurrency changed")
	}
}

func TestProviderAdmissionWaitCancelTimeoutAndQueueBound(t *testing.T) {
	for _, action := range []string{"release", "cancel", "timeout", "queue-full"} {
		t.Run(action, func(t *testing.T) {
			s := providerAdmissionServer(1, 50)
			tracker := s.policy.tracker
			if !tracker.tryReserveAccountSlot("owner", "account", 1) {
				t.Fatal("reserve owner")
			}
			if action == "queue-full" {
				for i := 0; i < defaultAccountConcurrencyMaxWaiting; i++ {
					tracker.tryBeginAccountWait(defaultAccountConcurrencyMaxWaiting)
				}
			}
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			c, w := providerAdmissionContext(ctx, "key", "account")
			done := make(chan bool, 1)
			go func() {
				release, ok := s.admitProviderGateway(c, providerGatewayBackoffKey{"account", "gateway", "model"})
				release()
				done <- ok
			}()
			if action == "release" || action == "cancel" {
				deadline := time.Now().Add(time.Second)
				for {
					tracker.mu.Lock()
					n := tracker.accountWaiters
					tracker.mu.Unlock()
					if n == 1 {
						break
					}
					if time.Now().After(deadline) {
						t.Fatal("did not enter wait queue")
					}
					time.Sleep(time.Millisecond)
				}
				if action == "release" {
					tracker.releaseAccountSlots("owner")
				} else {
					cancel()
				}
			}
			select {
			case allowed := <-done:
				if allowed != (action == "release") {
					t.Fatalf("admission=%v", allowed)
				}
			case <-time.After(time.Second):
				t.Fatal("wait did not end")
			}
			if action == "timeout" || action == "queue-full" {
				if w.Code != 429 {
					t.Fatalf("status=%d", w.Code)
				}
			}
			if action == "queue-full" {
				for i := 0; i < defaultAccountConcurrencyMaxWaiting; i++ {
					tracker.endAccountWait()
				}
			}
			tracker.releaseAccountSlots("owner")
			if tracker.accountWaiters != 0 || len(tracker.accountSlots) != 0 {
				t.Fatal("waiter or request leaked")
			}
		})
	}
}

func TestProviderGatewayRetryAfterPreservesFirstResponseAndSuppressesNext(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, status := range []int{429, 503} {
		var calls atomic.Int32
		upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			calls.Add(1)
			w.Header().Set("Retry-After", "30")
			w.Header().Set("X-Request-Id", "upstream-id")
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(status)
			_, _ = io.WriteString(w, `{"error":"upstream original"}`)
		}))
		gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "test", WireAPI: "responses", UpstreamModels: []string{"test-model"}, UpstreamModel: "test-model"}
		s := providerAdmissionServer(1, 0)
		s.manifest.Locale = "zh-CN"
		s.manifest.GatewayErrorMessages = map[string]map[string]string{"provider_retry_after": {"zh-CN": "等待 {{seconds}} 秒"}}
		for request := 0; request < 2; request++ {
			c, w := providerAdmissionContext(context.Background(), "key", "account")
			s.handleProviderGatewayRequest(c, gateway, []byte(`{"model":"test-model","input":"hi"}`), "test-model", sdktranslator.FormatOpenAIResponse, "")
			if request == 0 {
				if w.Code != status || w.Body.String() != `{"error":"upstream original"}` || w.Header().Get("X-Request-Id") != "upstream-id" {
					t.Fatalf("first response lost: %d %s %v", w.Code, w.Body.String(), w.Header())
				}
			} else if w.Code != 429 || !strings.Contains(w.Body.String(), "provider_retry_after") || !strings.Contains(w.Body.String(), "等待") {
				t.Fatalf("backoff not returned: %d %s", w.Code, w.Body.String())
			}
			if w.Header().Get("Retry-After") != "30" {
				t.Fatal("Retry-After lost")
			}
		}
		if calls.Load() != 1 || s.policy.tracker.accountInFlightCount("account") != 0 {
			t.Fatal("duplicate upstream request or leaked slot")
		}
		upstream.Close()
	}
}

func TestProviderGatewayStreamingHoldsSlotUntilBodyFinishes(t *testing.T) {
	gin.SetMode(gin.TestMode)
	finish := make(chan struct{})
	var once sync.Once
	releaseUpstream := func() { once.Do(func() { close(finish) }) }
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "data: first\n\n")
		w.(http.Flusher).Flush()
		select {
		case <-finish:
			_, _ = io.WriteString(w, "data: last\n\n")
		case <-r.Context().Done():
		}
	}))
	defer upstream.Close()
	defer releaseUpstream()
	s := providerAdmissionServer(1, 0)
	gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: "test", WireAPI: "responses", UpstreamModel: "test-model", UpstreamModels: []string{"test-model"}}
	body := []byte(`{"model":"test-model","input":"hi","stream":true}`)
	downstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, _ := gin.CreateTestContext(w)
		c.Request = r.WithContext(bindProviderGatewayAccount(r.Context(), "account"))
		s.handleProviderGatewayRequest(c, gateway, body, "test-model", sdktranslator.FormatOpenAIResponse, "")
	}))
	defer downstream.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, http.MethodPost, downstream.URL, nil)
	resp, err := downstream.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	reader := bufio.NewReader(resp.Body)
	if _, err = reader.ReadString('\n'); err != nil {
		t.Fatal(err)
	}
	c, w := providerAdmissionContext(context.Background(), "other-key", "account")
	s.handleProviderGatewayRequest(c, gateway, body, "test-model", sdktranslator.FormatOpenAIResponse, "")
	if w.Code != 429 {
		t.Fatalf("stream released slot early: %d", w.Code)
	}
	releaseUpstream()
	if _, err = io.ReadAll(reader); err != nil {
		t.Fatal(err)
	}
	if s.policy.tracker.accountInFlightCount("account") != 0 {
		t.Fatal("stream slot leaked")
	}
}
