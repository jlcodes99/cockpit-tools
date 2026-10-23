package main

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"testing"
	"time"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestDiagnosticAccountIdentitySurvivesDisabledBodiesAndAccountChanges(t *testing.T) {
	m := gatewayEndpointManifest(t)
	o := newRequestDiagnosticsObserver(m, "identity", time.Now(), false, nil)
	headers := http.Header{}
	headers.Set("Session-Id", "client-session")
	headers.Set("X-Codex-Installation-Id", "client-installation")
	headers.Set("Chatgpt-Account-Id", "client-workspace")
	headers.Set("Authorization", "Bearer never-record-this")
	headers.Set("Cookie", "never-record-this")
	o.ClientPayload(context.Background(), []byte(`{"token":"never-record-this"}`), headers, "http")
	for _, id := range []string{"endpoint-a", "endpoint-b"} {
		profile := diagnosticProfileForAccount(m.accountByID[id])
		upstream := http.Header{}
		upstream.Set("Session-Id", "session-"+id)
		upstream.Set("Thread-Id", "thread-"+id)
		upstream.Set("X-Codex-Installation-Id", profile.InstallationID)
		upstream.Set("Chatgpt-Account-Id", "workspace-"+id)
		upstream.Set("Authorization", "Bearer never-record-this")
		o.UpstreamRequest(context.Background(), cliproxyexecutor.DiagnosticRequest{AuthID: id, Transport: "http", Headers: upstream})
		upstream.Set("X-Codex-Installation-Id", "changed-after-send")
	}
	// Later runtime configuration cannot rewrite an earlier attempt's snapshot.
	m.Accounts[0].GatewayIdentity = m.Accounts[1].GatewayIdentity
	payload := o.base.finish(context.Background(), 200).payload
	if !payload.ClientIdentityCaptured || payload.ClientSessionID != "client-session" || !payload.ClientIdentityMetadataCaptured || payload.ClientInstallationID != "client-installation" || payload.ClientChatGPTAccountID != "client-workspace" {
		t.Fatal("client identity lost without body logging", payload)
	}
	if len(payload.Payloads) != 0 || len(o.base.rawPayloads) != 0 || len(payload.Attempts) != 2 {
		t.Fatal("identity capture enabled body retention or lost attempts")
	}
	for _, attempt := range payload.Attempts {
		if !attempt.UpstreamIdentityCaptured || !attempt.UpstreamIdentityMetadataCaptured || attempt.AccountIdentity == nil || attempt.UpstreamInstallationID != attempt.AccountIdentity.InstallationID || attempt.UpstreamChatGPTAccountID != "workspace-"+attempt.AccountID {
			t.Fatal("upstream snapshot does not match selected account", attempt)
		}
	}
	if payload.Attempts[0].AccountIdentity.InstallationID == payload.Attempts[1].AccountIdentity.InstallationID {
		t.Fatal("account profiles were mixed")
	}
	encoded, err := json.Marshal(payload)
	if err != nil || strings.Contains(string(encoded), "never-record-this") || strings.Contains(string(encoded), "changed-after-send") {
		t.Fatal("diagnostic identity retained credentials or mutable request headers", err)
	}
}

func TestDiagnosticWebsocketIdentityIsCapturedWithoutRetainingRawHeaders(t *testing.T) {
	m := gatewayEndpointManifest(t)
	o := newRequestDiagnosticsObserver(m, "websocket-identity", time.Now(), true, nil)
	headers := http.Header{"Session-Id": {"parent-session"}, "X-Codex-Installation-Id": {"client-installation"}, "Authorization": {"Bearer never-record-this"}}
	o.ClientPayload(context.Background(), []byte(`{"input":"never-record-this"}`), headers, "websocket")
	for _, id := range []string{"endpoint-a", "endpoint-b"} {
		o.ExecutionStarted(context.Background(), cliproxyexecutor.DiagnosticExecution{AuthID: id, Model: "m", StartedAt: time.Now()})
		profile := diagnosticProfileForAccount(m.accountByID[id])
		upstream := http.Header{"X-Codex-Installation-Id": {profile.InstallationID}}
		o.UpstreamRequest(context.Background(), cliproxyexecutor.DiagnosticRequest{AuthID: id, Transport: "websocket", Headers: upstream})
		payload := o.collector(context.Background()).finish(context.Background(), 200).payload
		if !payload.ClientIdentityMetadataCaptured || payload.ClientInstallationID != "client-installation" || payload.ClientSessionID != "parent-session" || len(payload.Attempts) != 1 || payload.Attempts[0].UpstreamInstallationID != profile.InstallationID {
			t.Fatal("websocket turn did not retain its own identity", payload)
		}
		if payload.Attempts[0].AccountIdentity.Scope != profile.Scope {
			t.Fatal("websocket turn reused a previous account profile")
		}
	}
	if len(o.clientHeader) != 0 || len(o.clientBody) != 0 {
		t.Fatal("disabled body logging retained raw client headers or body")
	}
}

func TestDiagnosticIdentityMissingInvalidAndOversizedValues(t *testing.T) {
	absent := captureDiagnosticIdentity(nil)
	if !absent.captured || !absent.metadataCaptured || absent.installation != "" {
		t.Fatal("absent header must be distinguishable from uncaptured history")
	}
	for _, bad := range []string{"bad\tvalue", strings.Repeat("x", 4097), string([]byte{255})} {
		headers := http.Header{"Session-Id": {"valid-session"}, "X-Codex-Installation-Id": {bad}}
		got := captureDiagnosticIdentity(headers)
		if !got.captured || got.session != "valid-session" || got.metadataCaptured || got.installation != "" || got.account != "" {
			t.Fatal("invalid metadata damaged session headers or was truncated into an identity")
		}
	}
	alias := captureDiagnosticIdentity(http.Header{"Session_id": {"alias"}})
	if alias.session != "alias" {
		t.Fatal("legacy session header was lost")
	}
}
