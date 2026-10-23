package main

import (
	"net/http"
	"unicode"
	"unicode/utf8"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
)

type diagnosticAccountIdentity = helps.ManagedIdentity

// Only these explicit identity fields may survive disabled body recording.
// Keep independent capture flags so older Session/Thread records cannot imply
// that installation metadata was observed too.
type diagnosticHeaderIdentity struct {
	captured, metadataCaptured             bool
	session, thread, installation, account string
}

func captureDiagnosticIdentity(headers http.Header) diagnosticHeaderIdentity {
	valid := func(value string) bool {
		if len(value) > 4096 || !utf8.ValidString(value) {
			return false
		}
		for _, ch := range value {
			if unicode.IsControl(ch) {
				return false
			}
		}
		return true
	}
	var result diagnosticHeaderIdentity
	session, thread := headers.Get("Session-Id"), headers.Get("Thread-Id")
	if session == "" {
		session = headers.Get("Session_id")
	}
	if valid(session) && valid(thread) {
		result.captured, result.session, result.thread = true, session, thread
	}
	installation, account := headers.Get("X-Codex-Installation-Id"), headers.Get("Chatgpt-Account-Id")
	if valid(installation) && valid(account) {
		result.metadataCaptured, result.installation, result.account = true, installation, account
	}
	return result
}

func diagnosticProfileForAccount(account *accountSpec) *diagnosticAccountIdentity {
	if account == nil {
		return nil
	}
	// Parse into an owned four-field snapshot now; never retain auth maps or look
	// up a current account profile when a historical record is displayed.
	profile, err := helps.ParseManagedIdentity(account.GatewayIdentity)
	if err != nil {
		return nil
	}
	return profile
}

func (o *requestDiagnosticsObserver) captureClientIdentity(headers http.Header) {
	identity := captureDiagnosticIdentity(headers)
	if o.websocket {
		o.mu.Lock()
		o.clientIdentity = identity
		o.mu.Unlock()
	}
	o.base.mu.Lock()
	o.base.clientIdentity = identity
	o.base.truncated = o.base.truncated || !identity.captured || !identity.metadataCaptured
	o.base.mu.Unlock()
}
