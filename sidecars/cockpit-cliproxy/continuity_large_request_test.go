package main

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestContinuityClientAcceptsConversationBeyondFormerEnvelopeLimit(t *testing.T) {
	const formerLimit = 64 << 20
	body := make([]byte, 0, formerLimit+128)
	body = append(body, `{"input":[{"role":"user","content":"`...)
	body = append(body, bytes.Repeat([]byte("x"), formerLimit)...)
	body = append(body, `"}]}`...)
	token := strings.Repeat("d", 64)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/continuity/bind" || r.Header.Get("Authorization") != "Bearer "+token {
			t.Error("lost authenticated local bind boundary")
		}
		if r.ContentLength <= formerLimit {
			t.Errorf("fixture did not exceed the old request limit: %d", r.ContentLength)
		}
		var request coreexecutor.ContinuityBindRequest
		if err := json.NewDecoder(r.Body).Decode(&request); err != nil {
			t.Error("large request could not be decoded", err)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		if request.AccountID != "local-a" || request.OwnerID != "owner-a" || !bytes.Equal(request.Body, body) {
			t.Error("large request lost its selected account, owner or history")
		}
		w.Header().Set("Content-Type", "application/json")
		// The returned body also exceeds the old response limit. Both directions
		// must retain the complete conversation instead of truncating the envelope.
		if err := json.NewEncoder(w).Encode(coreexecutor.ContinuityBindResponse{
			Ticket: request.Ticket, AttemptID: request.AttemptID, Body: request.Body,
			Headers: http.Header{"Session-Id": {"mapped-large-session"}},
		}); err != nil {
			t.Error("large response could not be encoded", err)
		}
	}))
	defer server.Close()
	client, err := newContinuityClient(server.URL, token, continuityManifestFixture(t))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	result, err := client.Bind(context.Background(), coreexecutor.ContinuityBindRequest{
		Ticket: "large-ticket", AttemptID: "large-attempt", AccountID: "sdk-a", Body: body,
	})
	if err != nil {
		t.Fatal("valid large conversation was rejected by the local bridge", err)
	}
	if result.Ticket != "large-ticket" || result.AttemptID != "large-attempt" || result.Headers.Get("Session-Id") != "mapped-large-session" || !bytes.Equal(result.Body, body) {
		t.Fatal("large bind response lost its history or continuation metadata")
	}
}
