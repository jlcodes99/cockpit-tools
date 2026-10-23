package helps

import (
	"fmt"
	"strings"
	"testing"
)

func TestTerminalResponseIDsAreBoundedAndIgnoreInvalidEvents(t *testing.T) {
	var ledger TerminalResponseIDs
	for i := 0; i < TerminalResponseIDLimit+10; i++ {
		ledger.Add(fmt.Sprint(i))
	}
	if len(ledger.ids) != TerminalResponseIDLimit || len(ledger.order) != TerminalResponseIDLimit || ledger.Contains("0") || !ledger.Contains("137") {
		t.Fatalf("unbounded/incorrect ledger: %v", ledger.order)
	}
	ledger.Add(strings.Repeat("x", 257))
	ledger.Add("")
	if len(ledger.ids) != TerminalResponseIDLimit {
		t.Fatal("invalid ID retained")
	}
	for _, payload := range []string{`{"type":"error","response":{"id":"wrong"}}`, `{"type":"response.done","response":{"id":"wrong","status":"in_progress"}}`} {
		if TerminalResponseID([]byte(payload)) != "" {
			t.Fatal("nonterminal event recorded")
		}
	}
	if TerminalResponseID([]byte(`{"type":"response.incomplete","response":{"id":"done"}}`)) != "done" {
		t.Fatal("incomplete is terminal")
	}
	ledger.Reset()
	if ledger.Contains("137") {
		t.Fatal("connection replacement retained IDs")
	}
}
