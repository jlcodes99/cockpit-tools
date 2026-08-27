// Package codebuddy provides authentication and token management functionality
// for Tencent CodeBuddy AI services. CodeBuddy subscriptions authenticate via
// OAuth against www.codebuddy.ai (international) or www.codebuddy.cn (China),
// while inference requests are sent to the OpenAI-compatible backend.
//
// Unlike other providers, CodeBuddy credentials are injected externally by the
// host application (the Tauri orchestrator completes OAuth login and writes the
// auth files directly). This package therefore only needs to persist and expose
// those credentials; it does not drive a login flow.
package codebuddy

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/misc"
)

// CodebuddyTokenStorage stores OAuth2 token information for CodeBuddy API
// authentication. Field names mirror the JSON contract written by the Rust
// orchestrator into the sidecar's auths/ directory.
type CodebuddyTokenStorage struct {
	// AccessToken is the OAuth2 access token used for authenticating API requests.
	AccessToken string `json:"access_token"`
	// RefreshToken is used to obtain new access tokens when the current one expires.
	RefreshToken string `json:"refresh_token"`
	// UID is the CodeBuddy user identifier.
	UID string `json:"uid,omitempty"`
	// EnterpriseID is the CodeBuddy enterprise/tenant identifier.
	EnterpriseID string `json:"enterprise_id,omitempty"`
	// Domain is the login domain (e.g. www.codebuddy.cn / www.codebuddy.ai).
	Domain string `json:"domain,omitempty"`
	// BaseURL is the inference backend base URL (e.g. https://copilot.tencent.com).
	BaseURL string `json:"base_url,omitempty"`
	// Email is the CodeBuddy account email.
	Email string `json:"email,omitempty"`
	// Region discriminates the site: "intl" or "cn".
	Region string `json:"region,omitempty"`
	// Type indicates the authentication provider type, always "codebuddy".
	Type string `json:"type"`
	// Expire is the timestamp when the current access token expires.
	Expire string `json:"expired,omitempty"`

	// Metadata holds arbitrary key-value pairs injected via hooks.
	// It is not exported to JSON directly to allow flattening during serialization.
	Metadata map[string]any `json:"-"`
}

// SetMetadata allows external callers to inject metadata into the storage before saving.
func (ts *CodebuddyTokenStorage) SetMetadata(meta map[string]any) {
	ts.Metadata = meta
}

// SaveTokenToFile serializes the CodeBuddy token storage to a JSON file.
// It creates the necessary directory structure and merges any injected metadata
// into the top-level JSON object, matching the pattern used by other providers.
func (ts *CodebuddyTokenStorage) SaveTokenToFile(authFilePath string) error {
	misc.LogSavingCredentials(authFilePath)
	ts.Type = "codebuddy"
	if err := os.MkdirAll(filepath.Dir(authFilePath), 0700); err != nil {
		return fmt.Errorf("failed to create directory: %v", err)
	}

	f, err := os.Create(authFilePath)
	if err != nil {
		return fmt.Errorf("failed to create token file: %w", err)
	}
	defer func() {
		_ = f.Close()
	}()

	data, errMerge := misc.MergeMetadata(ts, ts.Metadata)
	if errMerge != nil {
		return fmt.Errorf("failed to merge metadata: %w", errMerge)
	}

	if err = json.NewEncoder(f).Encode(data); err != nil {
		return fmt.Errorf("failed to write token to file: %w", err)
	}
	return nil
}
