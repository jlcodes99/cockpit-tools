package main

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func directConcurrencyRouter(upstreamURL, routeKind string, waitMS int) (*gin.Engine, *requestUsageTracker) {
	gateway := &providerGatewaySpec{BaseURL: upstreamURL, APIKey: "upstream-key", WireAPI: "responses", UpstreamModels: []string{"model-one", "model-two"}}
	account := &accountSpec{ID: "shared.account/one"}
	first := &apiKeySpec{ID: "first", Key: "first-key", Enabled: true, AccountIDs: []string{account.ID}}
	second := &apiKeySpec{ID: "second", Key: "second-key", Enabled: true, AccountIDs: []string{account.ID}}
	for _, spec := range []*apiKeySpec{first, second} {
		if routeKind == "fixed" {
			spec.ModelRouting = &modelRoutingSpec{
				DefaultRoute:  "oauth",
				FailurePolicy: "strict",
				Routes:        []modelRouteSpec{{ID: "direct", Namespace: "cpa", ProviderAccountID: account.ID, ProviderGateway: gateway}},
			}
		} else {
			spec.ProviderGateway = gateway
		}
	}
	loaded := &manifest{
		MaxAccountConcurrency: 1, AccountConcurrencyWaitMs: waitMS,
		APIKeys: []apiKeySpec{*first, *second}, Accounts: []accountSpec{*account},
		apiKeyByValue: map[string]*apiKeySpec{"first-key": first, "second-key": second},
		accountByID:   map[string]*accountSpec{account.ID: account},
	}
	tracker := newRequestUsageTracker()
	server := &relayServer{cfg: &config.Config{}, manifest: loaded, policy: &requestPolicy{manifest: loaded, tracker: tracker}}
	return server.router(), tracker
}

func directConcurrencyRequest(routeKind, key, model string, stream bool) *http.Request {
	if routeKind == "fixed" {
		model = "cpa/" + model
	}
	request := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(fmt.Sprintf(`{"model":%q,"input":"test","stream":%t}`, model, stream)))
	request.Header.Set("Authorization", "Bearer "+key)
	request.Header.Set("Content-Type", "application/json")
	return request
}

func assertDirectConcurrencyReleased(t *testing.T, tracker *requestUsageTracker) {
	t.Helper()
	tracker.mu.Lock()
	defer tracker.mu.Unlock()
	if tracker.accountWaiters != 0 || len(tracker.accountSlots) != 0 || len(tracker.accountInFlight) != 0 {
		t.Fatalf("account tracking leaked: waiters=%d requests=%d accounts=%d", tracker.accountWaiters, len(tracker.accountSlots), len(tracker.accountInFlight))
	}
}

func TestDirectProviderConcurrencyHoldsSlotUntilSSEEnds(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, routeKind := range []string{"direct", "fixed"} {
		t.Run(routeKind, func(t *testing.T) {
			var hits atomic.Int32
			started := make(chan struct{})
			release := make(chan struct{})
			var releaseOnce sync.Once
			unblock := func() { releaseOnce.Do(func() { close(release) }) }
			upstream := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				if hits.Add(1) == 1 {
					writer.Header().Set("Content-Type", "text/event-stream")
					fmt.Fprint(writer, "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"first\"}\n\n")
					writer.(http.Flusher).Flush()
					close(started)
					<-release
					fmt.Fprint(writer, "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"status\":\"completed\",\"output\":[]}}\n\n")
					return
				}
				writer.Header().Set("Content-Type", "application/json")
				fmt.Fprint(writer, `{"id":"resp_test","status":"completed","output":[]}`)
			}))
			t.Cleanup(upstream.Close)
			t.Cleanup(unblock)
			router, tracker := directConcurrencyRouter(upstream.URL, routeKind, 0)
			firstDone := make(chan *httptest.ResponseRecorder, 1)
			go func() {
				response := httptest.NewRecorder()
				router.ServeHTTP(response, directConcurrencyRequest(routeKind, "first-key", "model-one", true))
				firstDone <- response
			}()
			select {
			case <-started:
			case <-time.After(2 * time.Second):
				t.Fatal("first request did not reach upstream")
			}
			second := httptest.NewRecorder()
			router.ServeHTTP(second, directConcurrencyRequest(routeKind, "second-key", "model-two", false))
			if second.Code != http.StatusTooManyRequests || hits.Load() != 1 {
				t.Fatalf("shared account bypassed: status=%d upstream_hits=%d body=%s", second.Code, hits.Load(), second.Body.String())
			}
			unblock()
			select {
			case first := <-firstDone:
				if first.Code != http.StatusOK || !strings.Contains(first.Body.String(), "response.completed") {
					t.Fatalf("first stream failed: status=%d body=%s", first.Code, first.Body.String())
				}
			case <-time.After(2 * time.Second):
				t.Fatal("completed stream did not release its request")
			}
			third := httptest.NewRecorder()
			router.ServeHTTP(third, directConcurrencyRequest(routeKind, "second-key", "model-two", false))
			if third.Code != http.StatusOK || hits.Load() != 2 {
				t.Fatalf("released account unavailable: status=%d upstream_hits=%d", third.Code, hits.Load())
			}
			assertDirectConcurrencyReleased(t, tracker)
		})
	}
}

func TestDirectProviderConcurrencyWaitCancellationAndTimeout(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, outcome := range []string{"release", "cancel", "timeout"} {
		t.Run(outcome, func(t *testing.T) {
			var hits atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				hits.Add(1)
				writer.Header().Set("Content-Type", "application/json")
				fmt.Fprint(writer, `{"id":"resp_test","status":"completed","output":[]}`)
			}))
			defer upstream.Close()
			waitMS := 1000
			if outcome == "timeout" {
				waitMS = 80
			}
			router, tracker := directConcurrencyRouter(upstream.URL, "direct", waitMS)
			if !tracker.tryReserveAccountSlot("held", "cockpit-provider:shared.account/one", 1) {
				t.Fatal("failed to prepare occupied account")
			}
			defer tracker.releaseAccountSlots("held")
			requestContext, cancel := context.WithCancel(context.Background())
			defer cancel()
			request := directConcurrencyRequest("direct", "second-key", "model-two", false).WithContext(requestContext)
			done := make(chan *httptest.ResponseRecorder, 1)
			go func() {
				response := httptest.NewRecorder()
				router.ServeHTTP(response, request)
				done <- response
			}()
			deadline := time.Now().Add(2 * time.Second)
			for {
				tracker.mu.Lock()
				waiters := tracker.accountWaiters
				tracker.mu.Unlock()
				if waiters == 1 {
					break
				}
				select {
				case response := <-done:
					t.Fatalf("request bypassed waiting: status=%d upstream_hits=%d", response.Code, hits.Load())
				default:
				}
				if time.Now().After(deadline) {
					t.Fatal("request never registered as a waiter")
				}
				time.Sleep(time.Millisecond)
			}
			if outcome == "release" {
				tracker.releaseAccountSlots("held")
			} else if outcome == "cancel" {
				cancel()
				tracker.releaseAccountSlots("held")
			}
			select {
			case response := <-done:
				if outcome == "release" {
					if response.Code != http.StatusOK || hits.Load() != 1 {
						t.Fatalf("waited request failed: status=%d upstream_hits=%d", response.Code, hits.Load())
					}
				} else if hits.Load() != 0 {
					t.Fatalf("abandoned request reached upstream: outcome=%s hits=%d", outcome, hits.Load())
				} else if outcome == "timeout" && response.Code != http.StatusTooManyRequests {
					t.Fatalf("timeout status=%d, want 429", response.Code)
				}
			case <-time.After(2 * time.Second):
				t.Fatal("waiter did not finish")
			}
			tracker.releaseAccountSlots("held")
			assertDirectConcurrencyReleased(t, tracker)
		})
	}
}

func TestDirectProviderConcurrencyPreservesUpstreamBackoff(t *testing.T) {
	gin.SetMode(gin.TestMode)
	var hits atomic.Int32
	payload := `{"error":{"code":"rate_limit_exceeded","message":"capacity"}}`
	upstream := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		hits.Add(1)
		writer.Header().Set("Content-Type", "application/json")
		writer.Header().Set("Retry-After", "23")
		writer.Header().Set("X-CPA-Request-Id", "upstream-trace")
		writer.WriteHeader(http.StatusTooManyRequests)
		fmt.Fprint(writer, payload)
	}))
	defer upstream.Close()
	router, tracker := directConcurrencyRouter(upstream.URL, "direct", 1000)
	response := httptest.NewRecorder()
	router.ServeHTTP(response, directConcurrencyRequest("direct", "first-key", "model-one", false))
	if response.Code != http.StatusTooManyRequests || response.Body.String() != payload || response.Header().Get("Retry-After") != "23" || response.Header().Get("X-CPA-Request-Id") != "upstream-trace" || hits.Load() != 1 {
		t.Fatalf("upstream rejection changed: status=%d body=%s retry_after=%s trace=%s hits=%d", response.Code, response.Body.String(), response.Header().Get("Retry-After"), response.Header().Get("X-CPA-Request-Id"), hits.Load())
	}
	assertDirectConcurrencyReleased(t, tracker)
}
