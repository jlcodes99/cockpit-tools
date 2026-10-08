package main

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

func TestUsagePluginPreservesProxyRouteForHTTPAndWebSocket(t *testing.T) {
	for _, websocket := range []bool{false, true} {
		tracker := newRequestUsageTracker()
		plugin := &usagePlugin{tracker: tracker}
		ctx := internallogging.WithRequestID(context.Background(), "route-request")
		var result usagePayload
		if websocket {
			ctx = context.WithValue(ctx, websocketUsageContextKey,
				newWebsocketUsageSink("connection", func(payload usagePayload) { result = payload }))
		}
		plugin.HandleUsage(ctx, coreusage.Record{
			Model: "test", AuthID: "auth", RequestedAt: time.Unix(123, 0),
			ProxyRoute: &coreusage.ProxyRoute{Kind: "node", Name: "Tokyo 01"},
		})
		if !websocket {
			result, _ = tracker.finalize("route-request", usageFinalizeInput{status: 200})
		}
		if result.ProxyRoute == nil || result.ProxyRoute.Name != "Tokyo 01" {
			t.Fatalf("websocket=%v lost request route: %+v", websocket, result.ProxyRoute)
		}
		wire, err := json.Marshal(result)
		if err != nil {
			t.Fatal(err)
		}
		var decoded map[string]any
		if err = json.Unmarshal(wire, &decoded); err != nil {
			t.Fatal(err)
		}
		route := decoded["proxyRoute"].(map[string]any)
		if route["kind"] != "node" || route["name"] != "Tokyo 01" {
			t.Fatal(route)
		}
	}
}

func TestUsageProxyRouteAndAccountAttribution(t *testing.T) {
	route := func(name string) *coreusage.ProxyRoute {
		return &coreusage.ProxyRoute{Kind: "node", Name: name}
	}
	success := usagePayload{AccountID: "account-a", AccountEmail: "a@example.invalid", AuthID: "auth-a", Success: true,
		Usage: usageDetails{TotalTokens: 150}, ProxyRoute: route("node-a")}
	failure := usagePayload{AccountID: "account-b", AuthID: "auth-b", Success: false, ProxyRoute: route("node-b")}
	for _, tc := range []struct {
		name       string
		records    []usagePayload
		selected   string
		status     int
		wantID     string
		wantAuth   string
		wantRoute  string
		wantTokens int64
	}{
		{name: "successful_usage_before_later_selection", records: []usagePayload{success}, selected: "b", status: 200,
			wantID: "account-a", wantAuth: "auth-a", wantRoute: "node-a", wantTokens: 150},
		{name: "successful_usage_without_selection", records: []usagePayload{success}, status: 200,
			wantID: "account-a", wantAuth: "auth-a", wantRoute: "node-a", wantTokens: 150},
		{name: "final_failure_route_and_token_owner_differ", records: []usagePayload{success, failure}, selected: "b", status: 502,
			wantID: "account-a", wantAuth: "auth-a", wantRoute: "node-b", wantTokens: 150},
		{name: "final_failure_without_selection", records: []usagePayload{success, failure}, status: 502,
			wantID: "account-a", wantAuth: "auth-a", wantRoute: "node-b", wantTokens: 150},
		{name: "missing_usage_after_new_selection", records: []usagePayload{success}, selected: "b", status: 502,
			wantID: "account-a", wantAuth: "auth-a", wantTokens: 150},
		{name: "final_attempt_has_no_route", records: []usagePayload{success,
			{AccountID: "account-b", AuthID: "auth-b", Success: false}}, selected: "b", status: 502,
			wantID: "account-a", wantAuth: "auth-a", wantTokens: 150},
		{name: "same_account_final_route", records: []usagePayload{success,
			{AccountID: "account-a", AuthID: "auth-a", Success: false, ProxyRoute: route("retry-node")}}, selected: "a", status: 502,
			wantID: "account-a", wantAuth: "auth-a", wantRoute: "retry-node", wantTokens: 150},
		{name: "api_key_without_auth_keeps_own_account_and_route", records: []usagePayload{
			{AccountID: "api-account", Success: true, Usage: usageDetails{TotalTokens: 25}, ProxyRoute: route("api-node")}}, selected: "b", status: 200,
			wantID: "api-account", wantRoute: "api-node", wantTokens: 25},
		{name: "unmapped_api_key_does_not_borrow_selection", records: []usagePayload{
			{Success: true, Usage: usageDetails{TotalTokens: 25}, ProxyRoute: route("api-node")}}, selected: "b", status: 200,
			wantRoute: "api-node", wantTokens: 25},
		{name: "matching_auth_can_fill_account", records: []usagePayload{
			{AuthID: "auth-a", Success: true, Usage: usageDetails{TotalTokens: 25}, ProxyRoute: route("node-a")}}, selected: "a", status: 200,
			wantID: "account-a", wantAuth: "auth-a", wantRoute: "node-a", wantTokens: 25},
		{name: "selection_without_attempt", selected: "b", status: 502, wantID: "account-b", wantAuth: "auth-b"},
		{name: "never_connected", status: 400},
	} {
		t.Run(tc.name, func(t *testing.T) {
			tracker := newRequestUsageTracker()
			for _, record := range tc.records {
				record.RequestID = "request"
				tracker.record(record)
			}
			if tc.selected != "" {
				tracker.recordSelectedAccount("request", &accountSpec{ID: "account-" + tc.selected}, "auth-"+tc.selected)
			}
			payload, ok := tracker.finalize("request", usageFinalizeInput{status: tc.status})
			if !ok || payload.AccountID != tc.wantID || payload.AuthID != tc.wantAuth || payload.Usage.TotalTokens != tc.wantTokens {
				t.Fatalf("wrong usage owner or tokens: %+v", payload)
			}
			if tc.wantRoute == "" {
				if payload.ProxyRoute != nil {
					t.Fatalf("invented or borrowed route: %+v", payload.ProxyRoute)
				}
			} else if payload.ProxyRoute == nil || payload.ProxyRoute.Name != tc.wantRoute {
				t.Fatalf("want route %q, got %+v", tc.wantRoute, payload.ProxyRoute)
			}
			if payload.Success != (tc.status < 400) {
				t.Fatalf("wrong final status: %+v", payload)
			}
		})
	}
}
