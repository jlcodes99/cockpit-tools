import { isCodexApiKeyAccount, type CodexAccount } from '../types/codex';
import type { WorkbuddyAccount } from '../types/workbuddy';

/** Match the backend's credential-reference boundary, not a name or legacy plan label. */
export function codexWorkbuddySourceId(account: CodexAccount): string | null {
  if (!isCodexApiKeyAccount(account) || account.api_credential_kind !== 'workbuddy_acp') return null;
  return account.api_credential_ref?.trim() || null;
}

export function resolveCodexWorkbuddySource(
  account: CodexAccount,
  sources: readonly WorkbuddyAccount[],
): WorkbuddyAccount | undefined {
  const id = codexWorkbuddySourceId(account);
  return id ? sources.find(source => source.id === id) : undefined;
}

/** Keep only display inputs in the Codex view; never retain source credentials. */
export function workbuddyCreditDisplaySource(source: WorkbuddyAccount): WorkbuddyAccount {
  return {
    id: source.id,
    email: '',
    access_token: '',
    created_at: 0,
    last_used: 0,
    enterprise_id: source.enterprise_id,
    quota_raw: source.quota_raw,
    usage_raw: source.usage_raw,
    usage_updated_at: source.usage_updated_at,
    quota_query_last_error: source.quota_query_last_error,
  };
}
