package auth

import "testing"

func TestAuthKindClassifiesAPIKeyAndOAuth(t *testing.T) {
	t.Parallel()

	apiKeyAuth := &Auth{Attributes: map[string]string{"api_key": "sk-test"}}
	if got := apiKeyAuth.AuthKind(); got != AuthKindAPIKey {
		t.Fatalf("api key auth kind = %q, want %q", got, AuthKindAPIKey)
	}

	oauthAuth := &Auth{Metadata: map[string]any{"access_token": "tok", "email": "a@example.com"}}
	if got := oauthAuth.AuthKind(); got != AuthKindOAuth {
		t.Fatalf("oauth auth kind = %q, want %q", got, AuthKindOAuth)
	}

	explicit := &Auth{Attributes: map[string]string{"auth_kind": "access_token"}}
	if got := explicit.AuthKind(); got != AuthKindOAuth {
		t.Fatalf("access_token auth kind = %q, want %q", got, AuthKindOAuth)
	}
}
