import type { CodexLocalAccessAccountPoolHealth, CodexLocalAccessAccountPoolScopeDiagnostic } from '../types/codexLocalAccess';
import type { CodexAccount } from '../types/codex';

/** Selection counters describe the stage that blocked the request, not account credentials. */
export function codexAccountPoolDiagnosticReason(pool: CodexLocalAccessAccountPoolHealth) {
  const prefix = 'codex.localAccess.accountPoolHealth.dialog.';
  if (!pool.diagnosticAvailable) return { key: `${prefix}poolUnknownDetail`, values: {} };
  if (pool.candidateAuths === 0) return { key: `${prefix}poolNoCandidatesDetail`, values: {} };
  if (pool.candidateAuths > 0 && pool.scopedAuths === 0) {
    return { key: `${prefix}poolScopeMismatchDetail`, values: {} };
  }
  if (pool.scopedAuths > 0 && [pool.unavailableAuths, pool.modelExcludedAuths,
    pool.quotaReservedAuths, pool.imagePolicyBlockedAuths].some((count) => count > 0)) {
    return {
      key: `${prefix}poolBlockedDetail`,
      values: {
        unavailable: pool.unavailableAuths,
        modelExcluded: pool.modelExcludedAuths,
        quotaReserved: pool.quotaReservedAuths,
        imageBlocked: pool.imagePolicyBlockedAuths,
      },
    };
  }
  return { key: `${prefix}poolUnknownDetail`, values: {} };
}

export function codexAccountPoolFailureKey(pool: CodexLocalAccessAccountPoolHealth): string {
  return JSON.stringify([pool.apiKeyId, pool.lastFailureAt]);
}

export function codexAccountPoolInspectionLabel(pool: CodexLocalAccessAccountPoolHealth): string {
  const scopeFailure = (pool.scopeDiagnostics?.length ?? 0) > 0 ||
    (pool.diagnosticAvailable && pool.candidateAuths > 0 && pool.scopedAuths === 0);
  return `codex.localAccess.accountPoolHealth.dialog.${scopeFailure ? 'inspectBinding' : 'inspectConfig'}`;
}

export function codexAccountPoolScopeReasonKey(reasonCode: string): string {
  const keys: Record<string, string> = {
    scope_mismatch: 'scopeMismatchDetail',
    account_mapping_missing: 'accountMappingMissingDetail',
    bound_account_not_loaded: 'boundAccountNotLoadedDetail',
    bound_account_not_candidate: 'boundAccountNotCandidateDetail',
  };
  return `codex.localAccess.accountPoolHealth.dialog.${keys[reasonCode.trim().toLowerCase()] ?? 'scopeUnknownDetail'}`;
}

/** Credential identifiers are not account names; unknown mappings use a generic localized label. */
export function codexAccountPoolScopeDisplayName(
  diagnostic: CodexLocalAccessAccountPoolScopeDiagnostic,
  accounts: CodexAccount[],
  unknownLabel: string,
  accountReference?: (accountId: string) => string,
): string {
  if (diagnostic.reasonCode.trim().toLowerCase() === 'account_mapping_missing') return unknownLabel;
  const account = accounts.find((item) => item.id === diagnostic.accountId.trim());
  return account?.account_name?.trim() || account?.email?.trim() || diagnostic.accountEmail.trim() ||
    (diagnostic.accountId.trim() && accountReference ? accountReference(diagnostic.accountId.trim()) : unknownLabel);
}
