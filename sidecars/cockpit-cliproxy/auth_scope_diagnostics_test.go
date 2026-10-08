package main

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestScopeFailureDiagnosticsUseActualRequestScope(t *testing.T) {
	outside := &accountSpec{ID: "outside", Email: "outside@example.com"}
	bound := &accountSpec{ID: "bound", Email: "bound@example.com"}
	image := &accountSpec{ID: "image", Email: "image@example.com"}
	selector := &cockpitSelector{manifest: &manifest{
		accountByID:     map[string]*accountSpec{"outside": outside, "bound": bound, "image": image},
		accountByAuthID: map[string]*accountSpec{"auth-outside": outside, "auth-bound": bound},
	}}
	outsideAuth := &coreauth.Auth{ID: "auth-outside"}
	boundAuth := &coreauth.Auth{ID: "auth-bound"}
	unmapped := &coreauth.Auth{ID: "unmapped-auth"}
	for _, test := range []struct {
		name       string
		spec       *apiKeySpec
		kind       string
		candidates []*coreauth.Auth
		want       []authPoolScopeDiagnostic
	}{
		{
			name: "scope mismatch and bound candidate absent",
			spec: &apiKeySpec{AccountIDs: []string{" bound ", "bound"}}, candidates: []*coreauth.Auth{outsideAuth, outsideAuth},
			want: []authPoolScopeDiagnostic{
				{AccountID: "outside", AccountEmail: outside.Email, ReasonCode: "scope_mismatch"},
				{AccountID: "bound", AccountEmail: bound.Email, ReasonCode: "bound_account_not_candidate"},
			},
		},
		{
			name: "mapping missing and bound account not loaded",
			spec: &apiKeySpec{AccountIDs: []string{"absent", " absent "}}, candidates: []*coreauth.Auth{unmapped, nil, unmapped},
			want: []authPoolScopeDiagnostic{
				{ReasonCode: "account_mapping_missing"},
				{AccountID: "absent", ReasonCode: "bound_account_not_loaded"},
			},
		},
		{
			name: "empty candidates still explain bound state",
			spec: &apiKeySpec{AccountIDs: []string{"absent", "bound"}},
			want: []authPoolScopeDiagnostic{
				{AccountID: "absent", ReasonCode: "bound_account_not_loaded"},
				{AccountID: "bound", AccountEmail: bound.Email, ReasonCode: "bound_account_not_candidate"},
			},
		},
		{
			name: "image scope overrides conversation scope", kind: "image_generation",
			spec: &apiKeySpec{AccountIDs: []string{"bound"}, ImageGenerationAccountIDs: []string{" image ", "image"}}, candidates: []*coreauth.Auth{boundAuth},
			want: []authPoolScopeDiagnostic{
				{AccountID: "bound", AccountEmail: bound.Email, ReasonCode: "scope_mismatch"},
				{AccountID: "image", AccountEmail: image.Email, ReasonCode: "bound_account_not_candidate"},
			},
		},
		{
			name: "empty image scope uses conversation fallback", kind: "image_edit",
			spec: &apiKeySpec{AccountIDs: []string{"bound"}, ImageGenerationAccountIDs: []string{" "}}, candidates: []*coreauth.Auth{outsideAuth},
			want: []authPoolScopeDiagnostic{
				{AccountID: "outside", AccountEmail: outside.Email, ReasonCode: "scope_mismatch"},
				{AccountID: "bound", AccountEmail: bound.Email, ReasonCode: "bound_account_not_candidate"},
			},
		},
		{name: "no key", candidates: []*coreauth.Auth{unmapped}},
		{name: "no scope", spec: &apiKeySpec{}, candidates: []*coreauth.Auth{unmapped}},
		{name: "no scope and no candidates", spec: &apiKeySpec{}},
	} {
		t.Run(test.name, func(t *testing.T) {
			ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, test.spec)
			ctx = context.WithValue(ctx, requestKindContextKey, test.kind)
			if got := selector.diagnoseAPIKeyScope(ctx, test.candidates); !reflect.DeepEqual(got, test.want) {
				t.Fatalf("scope diagnostics = %#v, want %#v", got, test.want)
			}
			filtered := selector.filterAuthsForAPIKeyScope(ctx, test.candidates)
			if len(test.want) > 0 && len(filtered) != 0 {
				t.Fatalf("diagnostic scope diverges from filtering: %#v", filtered)
			}
			if len(apiKeyScopeAccountIDs(ctx)) == 0 && !reflect.DeepEqual(filtered, test.candidates) {
				t.Fatalf("unrestricted scope must preserve candidates: %#v", filtered)
			}
		})
	}
}

func TestScopeFailureDiagnosticsEventsBothSelectionEntrypoints(t *testing.T) {
	account := &accountSpec{ID: "outside", Email: "outside@example.com", AuthID: "secret-auth-file", UpstreamAPIKey: "secret-upstream-key"}
	selector := &cockpitSelector{manifest: &manifest{
		accountByID: map[string]*accountSpec{"outside": account},
	}, emitter: &eventEmitter{}}
	spec := &apiKeySpec{ID: "key-id", Key: "secret-client-key", AccountIDs: []string{"bound"}}
	ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, spec)
	candidates := []*coreauth.Auth{
		{ID: "secret-auth-file", Attributes: map[string]string{"account_id": "outside", "api_key": "secret-upstream-key"}, Metadata: map[string]any{"access_token": "secret-access-token"}},
		{ID: "secret-unmapped-auth", Metadata: map[string]any{"refresh_token": "secret-refresh-token"}},
	}
	for _, entrypoint := range []string{"Pick", "ReportAuthSelectionFailure"} {
		for _, empty := range []bool{false, true} {
			name := entrypoint
			if empty {
				name += "/empty candidates"
			}
			t.Run(name, func(t *testing.T) {
				auths := candidates
				if empty {
					auths = nil
				}
				var resultErr error
				out := captureStdout(t, func() {
					if entrypoint == "Pick" {
						var selected *coreauth.Auth
						selected, resultErr = selector.Pick(ctx, "codex", "gpt-6.1-sol", cliproxyexecutor.Options{}, auths)
						if selected != nil {
							t.Fatal("scope failure must not select any account")
						}
					} else {
						resultErr = selector.ReportAuthSelectionFailure(ctx, "codex", "gpt-6.1-sol", auths, &coreauth.Error{Code: "auth_unavailable", Message: "no auth available"})
					}
				})
				var authErr *coreauth.Error
				if !errors.As(resultErr, &authErr) || authErr.Code != "auth_unavailable" {
					t.Fatalf("selection error changed: %v", resultErr)
				}
				var payload requestDiagnosticPayload
				if err := json.Unmarshal([]byte(out), &payload); err != nil {
					t.Fatalf("invalid event: %v\n%s", err, out)
				}
				want := []authPoolScopeDiagnostic{{AccountID: "bound", ReasonCode: "bound_account_not_loaded"}}
				if !empty {
					want = append([]authPoolScopeDiagnostic{
						{AccountID: "outside", AccountEmail: account.Email, ReasonCode: "scope_mismatch"},
						{ReasonCode: "account_mapping_missing"},
					}, want...)
				}
				if payload.Type != "auth_pool_result" || payload.ScopedAuths != 0 || payload.CandidateAuths != len(auths) || !reflect.DeepEqual(payload.ScopeDiagnostics, want) {
					t.Fatalf("unexpected failure event: %#v", payload)
				}
				if payload.AccountID != "" || payload.AuthID != "" || len(payload.AccountStatuses) > 0 {
					t.Fatalf("scope failure must retain pool identity: %#v", payload)
				}
				if strings.Contains(out, "secret-") {
					t.Fatalf("credential leaked in scope diagnostics: %s", out)
				}
			})
		}
	}
}

func TestScopeFailureDiagnosticsAbsentAfterMatchingOrWithoutScope(t *testing.T) {
	account := &accountSpec{ID: "bound"}
	selector := &cockpitSelector{manifest: &manifest{accountByID: map[string]*accountSpec{"bound": account}}, emitter: &eventEmitter{}}
	auths := []*coreauth.Auth{{ID: "bound", Status: coreauth.StatusDisabled}}
	for _, ids := range [][]string{nil, {"bound"}} {
		ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{AccountIDs: ids})
		for _, report := range []bool{false, true} {
			out := captureStdout(t, func() {
				if report {
					selector.ReportAuthSelectionFailure(ctx, "codex", "gpt-6.1-sol", auths, errors.New("no auth available"))
				} else {
					selector.Pick(ctx, "codex", "gpt-6.1-sol", cliproxyexecutor.Options{}, auths)
				}
			})
			var payload requestDiagnosticPayload
			if err := json.Unmarshal([]byte(out), &payload); err != nil {
				t.Fatal(err)
			}
			if strings.Contains(out, "scopeDiagnostics") || len(payload.AccountStatuses) != 1 || payload.AccountStatuses[0].ReasonCode != "disabled" {
				t.Fatalf("non-scope failure should keep only existing account diagnostics: %s", out)
			}
		}
	}
}

func TestScopeFailureErrorMessageUsesHostTranslations(t *testing.T) {
	selector := &cockpitSelector{locale: " JA-jp ", manifest: &manifest{GatewayErrorMessages: map[string]map[string]string{
		"pool_unavailable":        {"ja": "利用可能なアカウントなし"},
		"pool_diagnostic_detail":  {"ja": "{{model}}:{{candidate}}/{{scoped}}/{{available}}/{{unavailable}}/{{modelExcluded}}/{{quotaReserved}}/{{imageBlocked}}"},
		"scope_mismatch":          {"ja": "範囲外"},
		"account_mapping_missing": {"ja": "アカウントとの対応付けなし"},
	}}}
	stats := authPoolSelectionStats{
		candidateAuths: 7, scopedAuths: 0, availableAuths: 0, unavailableAuths: 1,
		modelExcludedAuths: 2, quotaReservedAuths: 3, imagePolicyBlockedAuths: 4,
		scopeDiagnostics: []authPoolScopeDiagnostic{
			{AccountID: "private-account-one", AccountEmail: "private-one@example.com", ReasonCode: "scope_mismatch"},
			{AccountID: "private-account-two", ReasonCode: "scope_mismatch"},
			{ReasonCode: "account_mapping_missing"},
			{AccountID: "private-bound-account", ReasonCode: "bound_account_not_loaded"},
			{AccountID: "private-other-bound", ReasonCode: "bound_account_not_candidate"},
		},
	}
	err := selector.poolUnavailableError("gpt-6.1-sol", stats, "no auth available")
	want := "利用可能なアカウントなし. gpt-6.1-sol:7/0/0/1/2/3/4 範囲外 アカウントとの対応付けなし bound_account_not_loaded bound_account_not_candidate"
	if err.Message != want {
		t.Fatalf("localized scope error = %q, want %q", err.Message, want)
	}
	base := authPoolUnavailableError(selector.locale, stats, "no auth available")
	if err.Code != base.Code || err.HTTPStatus != base.HTTPStatus || err.Retryable != base.Retryable {
		t.Fatalf("scope explanation changed error contract: %#v vs %#v", err, base)
	}
	if strings.Contains(err.Message, "private-") {
		t.Fatal("scope error message must not expose account or credential identifiers")
	}
	stats.scopeDiagnostics = nil
	if err := selector.poolUnavailableError("gpt-6.1-sol", stats, "no auth available"); !reflect.DeepEqual(err, authPoolUnavailableError(selector.locale, stats, "no auth available")) {
		t.Fatalf("non-scope error behavior changed: %#v", err)
	}
}

func TestScopeFailureErrorTranslationFallback(t *testing.T) {
	translations := map[string]map[string]string{"scope_mismatch": {"pt-br": "Fora do âmbito", "pt": "Fora do escopo", "en": "Outside scope"}}
	for _, test := range []struct {
		locale string
		want   string
	}{
		{locale: "PT-BR", want: "Fora do âmbito"},
		{locale: "pt-PT", want: "Fora do escopo"},
		{locale: "de", want: "Outside scope"},
		{locale: "", want: "Outside scope"},
	} {
		t.Run(test.locale, func(t *testing.T) {
			selector := &cockpitSelector{locale: test.locale, manifest: &manifest{GatewayErrorMessages: translations}}
			got, found := selector.localizedPoolErrorMessage("scope_mismatch")
			if got != test.want || !found {
				t.Fatalf("translation = %q/%t, want %q/true", got, found, test.want)
			}
			stats := authPoolSelectionStats{scopeDiagnostics: []authPoolScopeDiagnostic{{ReasonCode: "scope_mismatch"}}}
			base := authPoolUnavailableError(test.locale, stats, "no auth available")
			if err := selector.poolUnavailableError("gpt-6.1-sol", stats, "no auth available"); err.Message != base.Message+" "+test.want {
				t.Fatalf("missing header translations must preserve base message: %q", err.Message)
			}
		})
	}
	selector := &cockpitSelector{manifest: &manifest{GatewayErrorMessages: map[string]map[string]string{"scope_mismatch": {"en": " "}}}}
	if got, found := selector.localizedPoolErrorMessage("scope_mismatch"); got != "scope_mismatch" || found {
		t.Fatalf("missing translation should use machine code: %q/%t", got, found)
	}
}
