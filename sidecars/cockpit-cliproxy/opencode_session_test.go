package main

import (
	"net/http"
	"strings"
	"testing"
)

func TestEnsureOpenCodeSessionHeaderPreservesProvidedValue(t *testing.T) {
	h := make(http.Header)
	h.Set("X-OpenCode-Session", "client-session")
	ensureOpenCodeSessionHeader(h)
	if got := h.Get("X-OpenCode-Session"); got != "client-session" {
		t.Fatalf("session = %q, want client-session", got)
	}
}

func TestEnsureOpenCodeSessionHeaderDerivesStableIdentity(t *testing.T) {
	h := make(http.Header)
	h.Set("X-Session-ID", "conversation-42")
	ensureOpenCodeSessionHeader(h)
	want := h.Get("X-OpenCode-Session")
	if want != "conversation-42" {
		t.Fatalf("session = %q, want conversation-42", want)
	}
	h2 := make(http.Header)
	h2.Set("X-Session-ID", "conversation-42")
	ensureOpenCodeSessionHeader(h2)
	if got := h2.Get("X-OpenCode-Session"); got != want {
		t.Fatalf("session changed across turns: %q vs %q", got, want)
	}
}

func TestEnsureOpenCodeSessionHeaderFallbackIsOpaque(t *testing.T) {
	h := make(http.Header)
	ensureOpenCodeSessionHeader(h)
	got := h.Get("X-OpenCode-Session")
	if !strings.HasPrefix(got, "cockpit-") || strings.ContainsAny(got, "\r\n") {
		t.Fatalf("invalid fallback session %q", got)
	}
}

func TestEnsureOpenCodeSessionHeaderRejectsControlValue(t *testing.T) {
	h := make(http.Header)
	h.Set("X-OpenCode-Session", "bad\r\nvalue")
	ensureOpenCodeSessionHeader(h)
	if got := h.Get("X-OpenCode-Session"); !strings.HasPrefix(got, "cockpit-") || strings.ContainsAny(got, "\r\n") {
		t.Fatalf("session = %q, want safe fallback", got)
	}
}

func TestIsOpenCodeGoEndpoint(t *testing.T) {
	if !isOpenCodeGoEndpoint("https://opencode.ai/zen/go/v1") {
		t.Fatal("expected OpenCode endpoint")
	}
	if isOpenCodeGoEndpoint("https://api.example.com/v1") {
		t.Fatal("unexpected OpenCode endpoint match")
	}
}
