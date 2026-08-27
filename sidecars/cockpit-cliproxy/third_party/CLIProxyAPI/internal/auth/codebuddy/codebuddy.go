package codebuddy

import (
	"net/http"
	"strings"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

// Endpoint constants for the Tencent CodeBuddy backend.
//
// Inference uses an OpenAI-compatible chat completions endpoint. The China site
// routes inference to copilot.tencent.com while its OAuth endpoint stays on
// www.codebuddy.cn; the international site uses www.codebuddy.ai for both.
const (
	IntlBaseURL    = "https://www.codebuddy.ai"
	CNBaseURL      = "https://copilot.tencent.com"
	ChatPath       = "/v2/chat/completions"
	ModelsPath     = "/v2/enterprises/personal/models"
	// Image endpoints are OpenAI Images API compatible (verified 2026-08-19:
	// POST /v2/images/generations with model/prompt/n/size returns the standard
	// "Image model [...] route config not found" error when the account lacks an
	// image route, confirming the endpoint exists and parses OpenAI-style params).
	ImageGenerationsPath = "/v2/images/generations"
	ImageEditsPath       = "/v2/images/edits"
	IntlRefreshURL       = "https://www.codebuddy.ai/v2/plugin/auth/token/refresh"
	CNRefreshURL         = "https://www.codebuddy.cn/v2/plugin/auth/token/refresh"
)

// Region values.
const (
	RegionIntl = "intl"
	RegionCN   = "cn"
)

// Creds holds the CodeBuddy credentials extracted from an auth record.
type Creds struct {
	AccessToken  string
	RefreshToken string
	UID          string
	EnterpriseID string
	Domain       string
	BaseURL      string
	Region       string
}

// CredsFromAuth extracts CodeBuddy credentials from an auth record's metadata.
// Credentials are written by the host orchestrator as top-level JSON fields, so
// they land in Auth.Metadata after the file store reads the auth file.
func CredsFromAuth(a *cliproxyauth.Auth) Creds {
	var c Creds
	if a == nil {
		return c
	}
	if a.Metadata != nil {
		c.AccessToken = metaString(a.Metadata, "access_token")
		c.RefreshToken = metaString(a.Metadata, "refresh_token")
		c.UID = metaString(a.Metadata, "uid")
		c.EnterpriseID = metaString(a.Metadata, "enterprise_id")
		c.Domain = metaString(a.Metadata, "domain")
		c.BaseURL = metaString(a.Metadata, "base_url")
		c.Region = metaString(a.Metadata, "region")
	}
	// Fall back to attributes for API-key style accounts.
	if a.Attributes != nil {
		if c.AccessToken == "" {
			c.AccessToken = strings.TrimSpace(a.Attributes["access_token"])
		}
		if c.BaseURL == "" {
			c.BaseURL = strings.TrimSpace(a.Attributes["base_url"])
		}
	}
	return c
}

// ResolveBaseURL returns the inference base URL, defaulting by region when unset.
func (c Creds) ResolveBaseURL() string {
	if strings.TrimSpace(c.BaseURL) != "" {
		return strings.TrimRight(strings.TrimSpace(c.BaseURL), "/")
	}
	if strings.EqualFold(strings.TrimSpace(c.Region), RegionCN) {
		return CNBaseURL
	}
	return IntlBaseURL
}

// ResolveRefreshURL returns the token refresh URL, defaulting by region when unset.
func (c Creds) ResolveRefreshURL() string {
	if strings.EqualFold(strings.TrimSpace(c.Region), RegionCN) {
		return CNRefreshURL
	}
	return IntlRefreshURL
}

// ResolveModelsURL returns the model-list endpoint URL, defaulting by region
// when the base URL is unset.
func (c Creds) ResolveModelsURL() string {
	return c.ResolveBaseURL() + ModelsPath
}

// ApplyHeaders sets the headers required by the CodeBuddy backend on r.
// It mirrors the header set used by the inference executor so the model-list
// endpoint is authenticated identically to chat completions.
func ApplyHeaders(r *http.Request, c Creds) {
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("Accept", "application/json")
	if c.AccessToken != "" {
		r.Header.Set("Authorization", "Bearer "+c.AccessToken)
	}
	if c.UID != "" {
		r.Header.Set("X-User-Id", c.UID)
	}
	if c.EnterpriseID != "" {
		r.Header.Set("X-Enterprise-Id", c.EnterpriseID)
		r.Header.Set("X-Tenant-Id", c.EnterpriseID)
	}
	if c.Domain != "" {
		r.Header.Set("X-Domain", c.Domain)
	}
	r.Header.Set("X-Product", "SaaS")
	r.Header.Set("X-IDE-Name", "CodeBuddyIDE")
	r.Header.Set("X-Requested-With", "XMLHttpRequest")
	r.Header.Set("User-Agent", "CodeBuddyIDE")
}

func metaString(meta map[string]any, key string) string {
	if meta == nil {
		return ""
	}
	if v, ok := meta[key].(string); ok {
		return strings.TrimSpace(v)
	}
	return ""
}
