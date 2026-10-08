package util

import (
	"context"
	"net/http"
	"testing"
)

func TestV72157CustomHeaderExpandsCanonicalSessionID(t *testing.T) {
	req := (&http.Request{Header: make(http.Header)}).WithContext(WithSessionID(context.Background(), "codex:thread-123"))
	ApplyCustomHeadersFromAttrs(req, map[string]string{
		"header:X-Session":  "$CPA-SESSION-ID",
		"header:X-Combined": "prefix-$CPA-SESSION-ID-suffix",
	})
	if got := req.Header.Get("X-Session"); got != "codex:thread-123" {
		t.Fatalf("X-Session = %q", got)
	}
	if got := req.Header.Get("X-Combined"); got != "prefix-codex:thread-123-suffix" {
		t.Fatalf("X-Combined = %q", got)
	}
}
