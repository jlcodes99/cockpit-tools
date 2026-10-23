package handlers

import (
	"encoding/json"
	"net/http"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestTerminalAccountPauseDoesNotRequestReauthentication(t *testing.T) {
	for _, tc := range []struct {
		code, wantCode, wantType string
	}{
		{"account_paused", "account_paused", "invalid_request_error"},
		{"refresh_token_reused", "upstream_authentication_required", "authentication_error"},
		{"auth_unavailable", "upstream_authentication_required", "authentication_error"},
	} {
		t.Run(tc.code, func(t *testing.T) {
			err := coreauth.NewTerminalAuthError(&coreauth.Error{Code: tc.code, Message: "fixture reason", HTTPStatus: http.StatusServiceUnavailable}, nil)
			body := BuildErrorResponseBodyWithError(http.StatusServiceUnavailable, err.Error(), err)
			var response ErrorResponse
			if decodeErr := json.Unmarshal(body, &response); decodeErr != nil {
				t.Fatal(decodeErr)
			}
			if response.Error.Code != tc.wantCode || response.Error.Type != tc.wantType || response.Error.Retryable == nil || *response.Error.Retryable {
				t.Fatalf("terminal classification changed: %s", body)
			}
			if tc.code == "account_paused" && response.Error.Message != "fixture reason" {
				t.Fatalf("pause detail changed: %s", body)
			}
		})
	}
}
