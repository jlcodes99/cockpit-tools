package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/gin-gonic/gin"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func tierControlFixture(enabled bool) (*relayServer, *gin.Engine) {
	m := &manifest{ServiceTierControlEnabled: enabled, DiagnosticsControlKey: "private-control-key",
		APIKeys: []apiKeySpec{{ID: "public", Key: "public-key", Enabled: true},
			{ID: "provider", Key: "provider-key", Enabled: true, ProviderGateway: &providerGatewaySpec{}}}}
	m.apiKeyByValue = map[string]*apiKeySpec{"public-key": &m.APIKeys[0], "provider-key": &m.APIKeys[1]}
	policy := &requestPolicy{manifest: m, tracker: newRequestUsageTracker(), tokenLimiter: newAPIKeyTokenLimiter(m)}
	if enabled {
		policy.serviceTierDefault = newAPIServiceTierDefault("priority")
	}
	s := &relayServer{manifest: m, policy: policy, runtime: &fakeRuntime{}}
	return s, s.router()
}

func updateTier(router *gin.Engine, key, remote, body string) *httptest.ResponseRecorder {
	req := httptest.NewRequest(http.MethodPost, apiServiceTierDefaultPath, strings.NewReader(body))
	req.Header.Set("Authorization", "Bearer "+key)
	req.Header.Set("Content-Type", "application/json")
	req.RemoteAddr = remote
	response := httptest.NewRecorder()
	router.ServeHTTP(response, req)
	return response
}

func TestAPIServiceTierControlRejectsUnauthorizedAndInvalidUpdates(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, tc := range []struct {
		name, key, remote, body string
		enabled                 bool
		status                  int
	}{
		{"public key", "public-key", "127.0.0.1:1000", `{"speed":"standard"}`, true, 403},
		{"missing key", "", "127.0.0.1:1000", `{"speed":"standard"}`, true, 403},
		{"remote control", "private-control-key", "192.0.2.1:1000", `{"speed":"standard"}`, true, 403},
		{"provider gateway disabled", "private-control-key", "127.0.0.1:1000", `{"speed":"standard"}`, false, 404},
		{"invalid speed", "private-control-key", "127.0.0.1:1000", `{"speed":"ultrafast","revision":1}`, true, 400},
		{"missing revision", "private-control-key", "127.0.0.1:1000", `{"speed":"standard"}`, true, 400},
		{"invalid JSON", "private-control-key", "127.0.0.1:1000", `{`, true, 400},
		{"unknown field", "private-control-key", "127.0.0.1:1000", `{"speed":"standard","revision":1,"port":9}`, true, 400},
		{"trailing JSON", "private-control-key", "127.0.0.1:1000", `{"speed":"standard","revision":1}{}`, true, 400},
		{"oversized body", "private-control-key", "127.0.0.1:1000", `{"speed":"standard","revision":1}` + strings.Repeat(" ", 5000), true, 400},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s, router := tierControlFixture(tc.enabled)
			response := updateTier(router, tc.key, tc.remote, tc.body)
			if response.Code != tc.status {
				t.Fatalf("status=%d body=%s", response.Code, response.Body.String())
			}
			if tc.enabled && s.policy.serviceTierDefault.tier() != "priority" {
				t.Fatal("rejected control changed the last valid default")
			}
		})
	}
}

func TestAPIServiceTierControlChangesNewHTTPRequestsOnly(t *testing.T) {
	gin.SetMode(gin.TestMode)
	s, router := tierControlFixture(true)
	var captured context.Context
	router.GET("/test/snapshot", func(c *gin.Context) {
		captured = c.Request.Context()
		tier, present := coreexecutor.ServiceTierDefaultSnapshot(captured)
		c.JSON(200, gin.H{"tier": tier, "present": present})
	})
	request := func(key string, websocket bool) (string, bool) {
		req := httptest.NewRequest(http.MethodGet, "/test/snapshot", nil)
		req.Header.Set("Authorization", "Bearer "+key)
		if websocket {
			req.Header.Set("Upgrade", "websocket")
		}
		response := httptest.NewRecorder()
		router.ServeHTTP(response, req)
		var got struct {
			Tier    string
			Present bool
		}
		if err := json.Unmarshal(response.Body.Bytes(), &got); err != nil {
			t.Fatal(err)
		}
		return got.Tier, got.Present
	}
	if tier, ok := request("public-key", false); !ok || tier != "priority" {
		t.Fatal("initial HTTP snapshot missing")
	}
	inflight := captured
	for index, speed := range []string{"standard", "fast", "standard"} {
		response := updateTier(router, "private-control-key", "127.0.0.1:1000", fmt.Sprintf(`{"speed":%q,"revision":%d}`, speed, index+1))
		if response.Code != 200 || !strings.Contains(response.Body.String(), `"speed":"`+speed+`"`) {
			t.Fatalf("update did not acknowledge applied speed: %s", response.Body.String())
		}
		want := ""
		if speed == "fast" {
			want = "priority"
		}
		if tier, ok := request("public-key", false); !ok || tier != want {
			t.Fatalf("new request tier=%q present=%v", tier, ok)
		}
		if tier, ok := coreexecutor.ServiceTierDefaultSnapshot(coreexecutor.SnapshotServiceTierDefault(inflight)); !ok || tier != "priority" {
			t.Fatal("control changed an active request or its retry")
		}
	}
	if _, ok := request("provider-key", false); ok {
		t.Fatal("provider gateway inherited API-service defaults")
	}
	if _, ok := request("public-key", true); ok {
		t.Fatal("WS connection fixed the default before a turn")
	}
	connection := captured
	s.policy.serviceTierDefault.fast.Store(true)
	first := coreexecutor.SnapshotServiceTierDefault(connection)
	s.policy.serviceTierDefault.fast.Store(false)
	second := coreexecutor.SnapshotServiceTierDefault(connection)
	if tier, ok := coreexecutor.ServiceTierDefaultSnapshot(first); !ok || tier != "priority" {
		t.Fatal("first WS turn lost its snapshot")
	}
	if tier, ok := coreexecutor.ServiceTierDefaultSnapshot(second); !ok || tier != "" {
		t.Fatal("next WS turn retained stale Fast")
	}
}

func TestAPIServiceTierControlRejectsDelayedOldUpdate(t *testing.T) {
	s, router := tierControlFixture(true)
	for _, tc := range []struct {
		speed    string
		revision uint64
		status   int
	}{
		{"standard", 2, 200},
		{"fast", 1, 409},
		{"standard", 2, 200},
		{"fast", 2, 409},
		{"fast", 3, 200},
	} {
		response := updateTier(router, "private-control-key", "127.0.0.1:1000", fmt.Sprintf(`{"speed":%q,"revision":%d}`, tc.speed, tc.revision))
		if response.Code != tc.status {
			t.Fatalf("revision %d: HTTP %d", tc.revision, response.Code)
		}
		if tc.revision < 3 && s.policy.serviceTierDefault.tier() != "" {
			t.Fatal("stale update undid acknowledged Standard")
		}
	}
}

func TestAPIServiceTierDefaultConcurrentSwitchesAndSnapshots(t *testing.T) {
	state := newAPIServiceTierDefault("priority")
	connection := coreexecutor.WithServiceTierDefaultProvider(context.Background(), state.tier)
	var wg sync.WaitGroup
	for worker := 0; worker < 8; worker++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := 0; i < 200; i++ {
				state.fast.Store(i%2 == 0)
				request := coreexecutor.SnapshotServiceTierDefault(connection)
				a, ok := coreexecutor.ServiceTierDefaultSnapshot(request)
				b, again := coreexecutor.ServiceTierDefaultSnapshot(coreexecutor.SnapshotServiceTierDefault(request))
				if !ok || !again || a != b {
					t.Error("request snapshot changed concurrently")
				}
			}
		}()
	}
	wg.Wait()
}
