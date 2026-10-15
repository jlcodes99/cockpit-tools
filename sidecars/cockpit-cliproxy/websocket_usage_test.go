package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

func TestWebsocketUsageEachExecutionAndDuplicate(t *testing.T) {
	var emitted []usagePayload
	sink := newWebsocketUsageSink("connection", func(p usagePayload) { emitted = append(emitted, p) })
	ctx := context.WithValue(context.Background(), websocketUsageContextKey, sink)
	tracker := newRequestUsageTracker()
	plugin := &usagePlugin{tracker: tracker, manifest: &manifest{accountByAuthID: map[string]*accountSpec{
		"a": {ID: "workspace-a", Email: "same@example.invalid"},
		"b": {ID: "workspace-b", Email: "same@example.invalid"},
	}}}
	for i := 0; i < 12; i++ {
		auth := "a"
		if i >= 6 {
			auth = "b"
		}
		r := coreusage.Record{AuthID: auth, Model: "test-model", RequestedAt: time.Unix(100, int64(i)), Detail: coreusage.Detail{InputTokens: 100, CachedTokens: 20, OutputTokens: 10, TotalTokens: 110}}
		plugin.HandleUsage(ctx, r)
		plugin.HandleUsage(ctx, r)
	}
	if len(emitted) != 12 {
		t.Fatalf("want 12 immediately emitted executions, got %d", len(emitted))
	}
	ids := map[string]bool{}
	for i, p := range emitted {
		if ids[p.RequestID] {
			t.Fatal("execution ID collision")
		}
		ids[p.RequestID] = true
		want := "workspace-a"
		if i >= 6 {
			want = "workspace-b"
		}
		if p.AccountID != want || p.Usage.InputTokens != 100 {
			t.Fatalf("wrong attribution/usage: %+v", p)
		}
	}
	if len(tracker.records) != 0 {
		t.Fatal("websocket usage must not enter connection finalization")
	}
}

func TestWebsocketUsageLateCallbackAndFailure(t *testing.T) {
	var emitted []usagePayload
	sink := newWebsocketUsageSink("connection", func(p usagePayload) { emitted = append(emitted, p) })
	ctx, cancel := context.WithCancel(context.WithValue(context.Background(), websocketUsageContextKey, sink))
	cancel()
	plugin := &usagePlugin{tracker: newRequestUsageTracker()}
	plugin.HandleUsage(ctx, coreusage.Record{Model: "test-model", RequestedAt: time.Unix(100, 0), Failed: true, Detail: coreusage.Detail{InputTokens: 40, OutputTokens: 2, TotalTokens: 42}})
	if len(emitted) != 1 || emitted[0].Success || emitted[0].Usage.TotalTokens != 42 {
		t.Fatalf("lost late failure usage: %+v", emitted)
	}
}

func TestSSEUsageStillUsesExistingTracker(t *testing.T) {
	tracker := newRequestUsageTracker()
	plugin := &usagePlugin{tracker: tracker}
	ctx := internallogging.WithRequestID(context.Background(), "sse")
	plugin.HandleUsage(ctx, coreusage.Record{RequestedAt: time.Unix(100, 0), Detail: coreusage.Detail{InputTokens: 100}})
	p, ok := tracker.finalize("sse", usageFinalizeInput{status: 200})
	if !ok || p.Usage.InputTokens != 100 {
		t.Fatal("SSE finalization changed")
	}
}

func TestWebsocketUsageConcurrentDuplicate(t *testing.T) {
	n := 0
	sink := newWebsocketUsageSink("connection", func(usagePayload) { n++ })
	record := coreusage.Record{RequestedAt: time.Unix(100, 0)}
	var wg sync.WaitGroup
	for i := 0; i < 50; i++ {
		wg.Add(1)
		go func() { defer wg.Done(); sink.record(record, usagePayload{}) }()
	}
	wg.Wait()
	if n != 1 {
		t.Fatalf("duplicate billed %d times", n)
	}
}

func TestWebsocketUsageMissingIdentityDoesNotMerge(t *testing.T) {
	n := 0
	sink := newWebsocketUsageSink("connection", func(usagePayload) { n++ })
	for i := 0; i < 2; i++ {
		sink.record(coreusage.Record{}, usagePayload{})
	}
	if n != 2 {
		t.Fatal("unknown execution identities were merged")
	}
}

func TestWebsocketMiddlewareLateUsageNoClosingBill(t *testing.T) {
	gin.SetMode(gin.TestMode)
	tracker := newRequestUsageTracker()
	policy := &requestPolicy{tracker: tracker, emitter: &eventEmitter{}}
	plugin := &usagePlugin{tracker: tracker}
	router := gin.New()
	router.Use(policy.middleware())
	var saved context.Context
	router.GET("/v1/responses", func(ctx *gin.Context) { saved = ctx.Request.Context(); ctx.Status(http.StatusSwitchingProtocols) })
	req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
	req.Header.Set("Upgrade", "websocket")
	out := captureStdout(t, func() {
		router.ServeHTTP(httptest.NewRecorder(), req)
		if saved == nil {
			t.Fatal("handler not reached")
		}
		for i := 0; i < 3; i++ {
			plugin.HandleUsage(saved, coreusage.Record{Model: "test", RequestedAt: time.Unix(100, int64(i)), Detail: coreusage.Detail{InputTokens: 10, TotalTokens: 10}})
		}
	})
	count := 0
	for _, line := range strings.Split(out, "\n") {
		var e usagePayload
		if json.Unmarshal([]byte(line), &e) == nil && e.Type == "usage" {
			count++
			if e.Usage.InputTokens != 10 {
				t.Fatal("spurious connection-closing bill")
			}
		}
	}
	if count != 3 {
		t.Fatalf("want 3 per-execution events, got %d", count)
	}
}

func TestWebsocketRejectedHandshakeIsRecorded(t *testing.T) {
	for _, status := range []int{http.StatusBadRequest, http.StatusTooManyRequests} {
		t.Run(http.StatusText(status), func(t *testing.T) {
			gin.SetMode(gin.TestMode)
			tracker := newRequestUsageTracker()
			policy := &requestPolicy{tracker: tracker, emitter: &eventEmitter{}}
			router := gin.New()
			router.Use(policy.middleware())
			router.GET("/v1/responses", func(ctx *gin.Context) {
				if status == http.StatusTooManyRequests {
					policy.emitTokenLimitBlockedRequest(ctx, ensureRequestID(ctx), &apiKeySpec{ID: "test-key"}, "test-model", "responses", time.Now(), "test limit")
				}
				ctx.AbortWithStatus(status)
			})
			req := httptest.NewRequest(http.MethodGet, "/v1/responses", nil)
			req.Header.Set("Upgrade", "websocket")
			out := captureStdout(t, func() { router.ServeHTTP(httptest.NewRecorder(), req) })
			count := 0
			for _, line := range strings.Split(out, "\n") {
				var payload usagePayload
				if json.Unmarshal([]byte(line), &payload) != nil || payload.Type != "usage" {
					continue
				}
				count++
				if payload.Success || payload.Status != status || payload.Usage.TotalTokens != 0 {
					t.Fatalf("wrong failure record: %+v", payload)
				}
				if status == http.StatusTooManyRequests && payload.ErrorCategory != "token_limit_exceeded" {
					t.Fatalf("lost limit category: %+v", payload)
				}
			}
			if count != 1 {
				t.Fatalf("want one failure record, got %d", count)
			}
		})
	}
}

func TestWebsocketRealUpgradeHasNoClosingBill(t *testing.T) {
	gin.SetMode(gin.TestMode)
	policy := &requestPolicy{tracker: newRequestUsageTracker(), emitter: &eventEmitter{}}
	router := gin.New()
	router.Use(policy.middleware())
	done := make(chan struct{})
	router.GET("/v1/responses", func(ctx *gin.Context) {
		upgrader := websocket.Upgrader{}
		conn, err := upgrader.Upgrade(ctx.Writer, ctx.Request, nil)
		if err != nil {
			t.Error(err)
			return
		}
		conn.Close()
	})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		router.ServeHTTP(w, r)
		close(done)
	}))
	defer server.Close()
	out := captureStdout(t, func() {
		conn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(server.URL, "http")+"/v1/responses", nil)
		if err != nil {
			t.Fatal(err)
		}
		conn.Close()
		<-done
	})
	for _, line := range strings.Split(out, "\n") {
		var payload usagePayload
		if json.Unmarshal([]byte(line), &payload) == nil && payload.Type == "usage" {
			t.Fatal("real websocket upgrade produced a spurious closing bill")
		}
	}
}
