package executor

import (
	"net/http"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
)

// Official Codex client family used to keep User-Agent and Originator paired.
// Upstream /backend-api/codex 404s when originator does not match the UA first segment.
var codexOfficialClientUAPrefixes = []string{
	"codex_cli_rs/",
	"codex-tui/",
	"codex_vscode/",
	"codex_vscode_copilot/",
	"codex_app/",
	"codex_chatgpt_desktop/",
	"codex_atlas/",
	"codex_exec/",
	"codex_sdk_ts/",
}

const codexOfficialClientFamilyPrefix = "codex "

var codexOfficialClientOriginators = map[string]bool{
	"codex_cli_rs":          true,
	"codex-tui":             true,
	"codex_vscode":          true,
	"codex_vscode_copilot":  true,
	"codex_app":             true,
	"codex_chatgpt_desktop": true,
	"codex_atlas":           true,
	"codex_exec":            true,
	"codex_sdk_ts":          true,
}

func normalizeCodexClientHeader(value string) string {
	return strings.ToLower(strings.TrimSpace(value))
}

func isCodexOfficialClientOriginator(originator string) bool {
	value := normalizeCodexClientHeader(originator)
	if value == "" {
		return false
	}
	if codexOfficialClientOriginators[value] {
		return true
	}
	return strings.HasPrefix(value, codexOfficialClientFamilyPrefix)
}

func isSaneCodexOriginator(value string) bool {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" || len(trimmed) > 64 {
		return false
	}
	return !strings.ContainsAny(trimmed, "\r\n\t")
}

func canonicalizeCodexOriginator(value string) string {
	trimmed := strings.TrimSpace(value)
	normalized := normalizeCodexClientHeader(trimmed)
	if strings.HasPrefix(normalized, codexOfficialClientFamilyPrefix) {
		return trimmed
	}
	if official, ok := lookupCanonicalCodexOriginator(normalized); ok {
		return official
	}
	return trimmed
}

func lookupCanonicalCodexOriginator(normalized string) (string, bool) {
	for official := range codexOfficialClientOriginators {
		if official == normalized {
			return official, true
		}
	}
	return "", false
}

func codexUATrailerName(ua string) string {
	last := strings.LastIndex(ua, "(")
	if last < 0 {
		return ""
	}
	rest := ua[last+1:]
	closeIdx := strings.Index(rest, ")")
	if closeIdx < 0 {
		return ""
	}
	inner := strings.TrimSpace(rest[:closeIdx])
	if semi := strings.Index(inner, ";"); semi >= 0 {
		inner = strings.TrimSpace(inner[:semi])
	}
	return inner
}

// PairCodexClientIdentity derives a matching originator from the outbound User-Agent.
// Returns false when the UA is not an official Codex client family identity.
func PairCodexClientIdentity(userAgent string) (originator string, pairedUA string, ok bool) {
	ua := strings.TrimSpace(userAgent)
	slash := strings.IndexByte(ua, '/')
	if slash <= 0 {
		return "", "", false
	}
	if leading := strings.TrimSpace(ua[:slash]); isSaneCodexOriginator(leading) && isCodexOfficialClientOriginator(leading) {
		leading = canonicalizeCodexOriginator(leading)
		return leading, leading + ua[slash:], true
	}
	if trailer := codexUATrailerName(ua); trailer != "" && !strings.ContainsRune(trailer, '/') &&
		isSaneCodexOriginator(trailer) && isCodexOfficialClientOriginator(trailer) {
		trailer = canonicalizeCodexOriginator(trailer)
		return trailer, trailer + ua[slash:], true
	}
	return "", "", false
}

func codexCloakingEnabled(cfg *config.Config) bool {
	return cfg != nil && !cfg.Codex.DisableCodexCloaking
}

func applyCodexClientIdentityHeaders(headers, ginHeaders http.Header, cfg *config.Config, cfgUserAgent string, isAPIKey bool) {
	if headers == nil {
		return
	}
	if codexCloakingEnabled(cfg) {
		return
	}
	if isAPIKey {
		ensureHeaderWithPriority(headers, ginHeaders, "User-Agent", "", "")
	} else {
		ensureHeaderWithConfigPrecedence(headers, ginHeaders, "User-Agent", cfgUserAgent, codexUserAgent)
	}
	if originator := strings.TrimSpace(headerValueCaseInsensitive(ginHeaders, "Originator")); originator != "" {
		headers.Set("Originator", originator)
	} else if !isAPIKey && strings.TrimSpace(headers.Get("Originator")) == "" {
		headers.Set("Originator", codexOriginator)
	}
}

func applyPairedCodexClientIdentity(headers http.Header, forceOfficialFallback bool) {
	if headers == nil {
		return
	}
	ua := strings.TrimSpace(headers.Get("User-Agent"))
	if originator, pairedUA, ok := PairCodexClientIdentity(ua); ok {
		headers.Set("User-Agent", pairedUA)
		headers.Set("Originator", originator)
		return
	}
	if !forceOfficialFallback {
		return
	}
	headers.Set("User-Agent", codexUserAgent)
	headers.Set("Originator", codexOriginator)
}

func applyCanonicalCodexSessionHeader(headers http.Header, sessionID string) {
	if headers == nil {
		return
	}
	sessionID = strings.TrimSpace(sessionID)
	if sessionID == "" {
		return
	}
	setCodexSessionHeaderCasePreserved(headers, "Session-Id", sessionID)
}

func firstCodexSessionHeader(headers http.Header) string {
	if headers == nil {
		return ""
	}
	return codexSessionHeaderValue(headers)
}
