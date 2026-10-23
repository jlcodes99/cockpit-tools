package main

import (
	"context"
	"strconv"
	"strings"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

// authPoolScopeDiagnostic reports only identities known to the host manifest.
// In particular, unmapped credentials must never expose auth IDs or secrets.
type authPoolScopeDiagnostic struct {
	AccountID    string `json:"accountId"`
	AccountEmail string `json:"accountEmail,omitempty"`
	ReasonCode   string `json:"reasonCode"`
}

// poolUnavailableError keeps the original error contract, adding the exact
// scope causes using host-provided translations only on scope failures.
func (s *cockpitSelector) poolUnavailableError(model string, stats authPoolSelectionStats, detail string) *coreauth.Error {
	err := authPoolUnavailableError(s.locale, stats, detail)
	if len(stats.scopeDiagnostics) == 0 {
		return err
	}
	title, titleFound := s.localizedPoolErrorMessage("pool_unavailable")
	statsTemplate, statsFound := s.localizedPoolErrorMessage("pool_diagnostic_detail")
	if titleFound && statsFound {
		statsMessage := strings.NewReplacer(
			"{{model}}", model,
			"{{candidate}}", strconv.Itoa(stats.candidateAuths),
			"{{scoped}}", strconv.Itoa(stats.scopedAuths),
			"{{available}}", strconv.Itoa(stats.availableAuths),
			"{{unavailable}}", strconv.Itoa(stats.unavailableAuths),
			"{{modelExcluded}}", strconv.Itoa(stats.modelExcludedAuths),
			"{{quotaReserved}}", strconv.Itoa(stats.quotaReservedAuths),
			"{{imageBlocked}}", strconv.Itoa(stats.imagePolicyBlockedAuths),
		).Replace(statsTemplate)
		err.Message = title + ". " + statsMessage
	}
	seen := make(map[string]struct{}, len(stats.scopeDiagnostics))
	for _, diagnostic := range stats.scopeDiagnostics {
		message, _ := s.localizedPoolErrorMessage(diagnostic.ReasonCode)
		if _, exists := seen[message]; exists {
			continue
		}
		seen[message] = struct{}{}
		err.Message += " " + message
	}
	return err
}

func (s *cockpitSelector) localizedPoolErrorMessage(code string) (string, bool) {
	if s == nil || s.manifest == nil {
		return code, false
	}
	translations := s.manifest.GatewayErrorMessages[code]
	locale := strings.ToLower(strings.TrimSpace(s.locale))
	for _, candidate := range []string{locale, strings.Split(locale, "-")[0], "en"} {
		for language, message := range translations {
			if strings.EqualFold(language, candidate) && strings.TrimSpace(message) != "" {
				return strings.TrimSpace(message), true
			}
		}
	}
	return code, false
}

// apiKeyScopeAccountIDs is shared by filtering and failure diagnostics so image
// requests use the same image account pool, including the conversation fallback.
func apiKeyScopeAccountIDs(ctx context.Context) []string {
	if ctx == nil {
		return nil
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	if spec == nil {
		return nil
	}
	if requestKind, _ := ctx.Value(requestKindContextKey).(string); isImageRequestKind(requestKind) {
		if ids := imageGenerationAccountIDsForSpec(spec); len(ids) > 0 {
			return ids
		}
	}
	return spec.AccountIDs
}

// diagnoseAPIKeyScope is called only after scope filtering finds no match. It
// uses the already loaded manifest and this request's candidates without I/O.
// Output preserves candidate and configured-scope order and deduplicates rows.
func (s *cockpitSelector) diagnoseAPIKeyScope(ctx context.Context, candidates []*coreauth.Auth) []authPoolScopeDiagnostic {
	if s == nil || s.manifest == nil {
		return nil
	}
	ids := apiKeyScopeAccountIDs(ctx)
	if len(ids) == 0 {
		return nil
	}
	allowed := make(map[string]struct{}, len(ids))
	for _, id := range ids {
		if id = strings.TrimSpace(id); id != "" {
			allowed[id] = struct{}{}
		}
	}
	var diagnostics []authPoolScopeDiagnostic
	seen := make(map[authPoolScopeDiagnostic]struct{}, len(candidates)+len(ids))
	appendDiagnostic := func(account *accountSpec, id, reason string) {
		item := authPoolScopeDiagnostic{AccountID: id, ReasonCode: reason}
		if account != nil {
			item.AccountID = strings.TrimSpace(account.ID)
			item.AccountEmail = strings.TrimSpace(account.Email)
		}
		if _, exists := seen[item]; !exists {
			seen[item] = struct{}{}
			diagnostics = append(diagnostics, item)
		}
	}
	candidateAccounts := make(map[string]struct{}, len(candidates))
	for _, auth := range candidates {
		account := s.accountForAuth(auth)
		if account == nil {
			appendDiagnostic(nil, "", "account_mapping_missing")
			continue
		}
		candidateAccounts[account.ID] = struct{}{}
		if _, matches := allowed[account.ID]; !matches {
			appendDiagnostic(account, "", "scope_mismatch")
		}
	}
	for _, id := range ids {
		id = strings.TrimSpace(id)
		if id == "" {
			continue
		}
		account := s.manifest.accountByID[id]
		if account == nil {
			// Absence from this manifest does not prove that the host deleted it.
			appendDiagnostic(nil, id, "bound_account_not_loaded")
		} else if _, exists := candidateAccounts[account.ID]; !exists {
			// Candidate construction may exclude accounts for several reasons;
			// this layer cannot infer model support or credential validity.
			appendDiagnostic(account, "", "bound_account_not_candidate")
		}
	}
	return diagnostics
}
