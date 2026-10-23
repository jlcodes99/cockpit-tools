package executor

import (
	"net/http"
	"net/http/httptest"
	"testing"

	xaiauth "github.com/router-for-me/CLIProxyAPI/v7/internal/auth/xai"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestXAIClientVersionDynamicAndHeaderFloor(t *testing.T) {
	restore := helps.SetXAIClientVersionForTest("1.2.34")
	defer restore()
	for _, tc := range []struct{ name, override, want string }{
		{"dynamic", "", "1.2.34"},
		{"outdated", "0.2.93", "1.2.34"},
		{"below_floor", "1.0.12", "1.2.34"},
		{"malformed", "custom-client-version", "1.2.34"},
		{"prerelease", "1.3.0-alpha", "1.2.34"},
		{"supported_override", "1.0.13", "1.0.13"},
		{"new_override", "9.9.9", "9.9.9"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodPost, xaiauth.CLIChatProxyBaseURL+"/responses", nil)
			auth := &cliproxyauth.Auth{Attributes: map[string]string{
				"using_api": "false", "base_url": xaiauth.CLIChatProxyBaseURL,
				"header:" + xaiClientVersionHeader: tc.override,
				"header:User-Agent":                "xai-grok-workspace/0.2.93",
			}}
			applyXAIChatHeaders(req, auth, "test", true, "")
			if got := req.Header.Get(xaiClientVersionHeader); got != tc.want {
				t.Fatalf("version=%q, want %q", got, tc.want)
			}
			if got := req.Header.Get("User-Agent"); got != "xai-grok-workspace/"+tc.want {
				t.Fatalf("user-agent=%q, want version %q", got, tc.want)
			}
		})
	}
}

func TestXAIClientVersionHeaderReferenceCannotRestoreOldVersion(t *testing.T) {
	restore := helps.SetXAIClientVersionForTest("1.2.34")
	defer restore()
	req := httptest.NewRequest(http.MethodPost, xaiauth.CLIChatProxyBaseURL+"/responses", nil)
	auth := &cliproxyauth.Auth{Attributes: map[string]string{"using_api": "false", "base_url": xaiauth.CLIChatProxyBaseURL, "header:" + xaiClientVersionHeader: "$Client-Grok-Version"}}
	applyXAIChatHeaders(req, auth, "test", true, "", http.Header{"Client-Grok-Version": {"0.2.93"}})
	if got := req.Header.Get(xaiClientVersionHeader); got != "1.2.34" {
		t.Fatalf("inherited stale version=%q", got)
	}
}

func TestXAIClientVersionKnownIdentitiesStayConsistent(t *testing.T) {
	restore := helps.SetXAIClientVersionForTest("1.2.34")
	defer restore()
	for _, tc := range []struct {
		name, usingAPI, baseURL, version, legacyVersion, userAgent, wantVersion, wantLegacyVersion, wantUserAgent string
	}{
		{"imported CLI", "false", xaiauth.CLIChatProxyBaseURL, "0.2.93", "0.2.93", "grok-cli/0.2.93", "1.2.34", "1.2.34", "grok-cli/1.2.34"},
		{"alias only", "false", xaiauth.CLIChatProxyBaseURL, "", "0.2.93", "grok-cli/0.2.93", "1.2.34", "1.2.34", "grok-cli/1.2.34"},
		{"valid override", "false", xaiauth.CLIChatProxyBaseURL, "2.0.0", "0.2.93", "xai-grok-workspace/0.2.93", "2.0.0", "2.0.0", "xai-grok-workspace/2.0.0"},
		{"UA suffix", "false", xaiauth.CLIChatProxyBaseURL, "", "0.2.93", "grok-cli/0.2.93 (custom transport)", "1.2.34", "1.2.34", "grok-cli/1.2.34 (custom transport)"},
		{"unrelated UA", "false", xaiauth.CLIChatProxyBaseURL, "0.2.93", "0.2.93", "custom-agent/0.2.93", "1.2.34", "1.2.34", "custom-agent/0.2.93"},
		{"API key", "true", xaiauth.DefaultAPIBaseURL, "0.2.93", "0.2.93", "grok-cli/0.2.93", "0.2.93", "0.2.93", "grok-cli/0.2.93"},
		{"custom gateway", "false", "https://custom.example/v1", "0.2.93", "0.2.93", "grok-cli/0.2.93", "0.2.93", "0.2.93", "grok-cli/0.2.93"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodPost, tc.baseURL+"/responses", nil)
			auth := &cliproxyauth.Auth{Attributes: map[string]string{
				"using_api": tc.usingAPI, "base_url": tc.baseURL,
				"header:" + xaiClientVersionHeader:    tc.version,
				"header:" + xaiCLIClientVersionHeader: tc.legacyVersion,
				"header:User-Agent":                   tc.userAgent,
			}}
			applyXAIChatHeaders(req, auth, "test", true, "")
			for header, want := range map[string]string{xaiClientVersionHeader: tc.wantVersion, xaiCLIClientVersionHeader: tc.wantLegacyVersion, "User-Agent": tc.wantUserAgent} {
				if got := req.Header.Get(header); got != want {
					t.Errorf("%s=%q, want %q", header, got, want)
				}
			}
		})
	}
}

func TestXAIClientVersionDoesNotAddLegacyHeader(t *testing.T) {
	req := httptest.NewRequest(http.MethodPost, xaiauth.CLIChatProxyBaseURL+"/responses", nil)
	auth := &cliproxyauth.Auth{Attributes: map[string]string{"using_api": "false", "base_url": xaiauth.CLIChatProxyBaseURL}}
	applyXAIChatHeaders(req, auth, "test", true, "")
	if len(req.Header.Values(xaiCLIClientVersionHeader)) != 0 {
		t.Fatal("absent legacy header was injected")
	}
}
