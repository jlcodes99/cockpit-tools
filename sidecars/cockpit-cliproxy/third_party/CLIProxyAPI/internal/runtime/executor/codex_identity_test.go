package executor

import (
	"net/http"
	"testing"
)

func TestPairCodexClientIdentity(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name           string
		ua             string
		wantOriginator string
		wantUAPrefix   string
		ok             bool
	}{
		{
			name:           "official tui prefix",
			ua:             "codex-tui/0.146.0 (Mac OS 26.5.0; arm64) iTerm.app/3.6.10 (codex-tui; 0.146.0)",
			wantOriginator: "codex-tui",
			wantUAPrefix:   "codex-tui/",
			ok:             true,
		},
		{
			name:           "desktop family keeps Codex prefix",
			ua:             "Codex Desktop/0.146.0-alpha.3 (Mac OS 26.5.2; arm64) unknown (Codex Desktop; 26.721.30844)",
			wantOriginator: "Codex Desktop",
			wantUAPrefix:   "Codex Desktop/",
			ok:             true,
		},
		{
			name:           "override prefix recovered from trailer",
			ua:             "cccc/0.146.0 (Mac OS 26.5.0; arm64) iTerm.app/3.6.10 (codex-tui; 0.146.0)",
			wantOriginator: "codex-tui",
			wantUAPrefix:   "codex-tui/",
			ok:             true,
		},
		{
			name: "unofficial client is not paired",
			ua:   "curl/8.0",
			ok:   false,
		},
		{
			name:           "cli rs",
			ua:             "codex_cli_rs/0.146.0",
			wantOriginator: "codex_cli_rs",
			wantUAPrefix:   "codex_cli_rs/",
			ok:             true,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			originator, pairedUA, ok := PairCodexClientIdentity(test.ua)
			if ok != test.ok {
				t.Fatalf("ok = %v, want %v", ok, test.ok)
			}
			if !test.ok {
				return
			}
			if originator != test.wantOriginator {
				t.Fatalf("originator = %q, want %q", originator, test.wantOriginator)
			}
			if !stringsHasPrefix(pairedUA, test.wantUAPrefix) {
				t.Fatalf("paired UA = %q, want prefix %q", pairedUA, test.wantUAPrefix)
			}
		})
	}
}

func TestApplyPairedCodexClientIdentityFixesMismatch(t *testing.T) {
	t.Parallel()

	headers := http.Header{}
	headers.Set("User-Agent", "codex_cli_rs/0.146.0")
	headers.Set("Originator", "Codex Desktop")
	applyPairedCodexClientIdentity(headers, true)

	if got := headers.Get("Originator"); got != "codex_cli_rs" {
		t.Fatalf("Originator = %q, want codex_cli_rs", got)
	}
	if got := headers.Get("User-Agent"); got != "codex_cli_rs/0.146.0" {
		t.Fatalf("User-Agent = %q, want paired CLI UA", got)
	}
}

func TestApplyPairedCodexClientIdentityFallsBackToOfficial(t *testing.T) {
	t.Parallel()

	headers := http.Header{}
	headers.Set("User-Agent", "curl/8.0")
	headers.Set("Originator", "evil")
	applyPairedCodexClientIdentity(headers, true)

	if got := headers.Get("User-Agent"); got != codexUserAgent {
		t.Fatalf("User-Agent = %q, want official %q", got, codexUserAgent)
	}
	if got := headers.Get("Originator"); got != codexOriginator {
		t.Fatalf("Originator = %q, want official %q", got, codexOriginator)
	}
}

func stringsHasPrefix(value, prefix string) bool {
	return len(value) >= len(prefix) && value[:len(prefix)] == prefix
}
