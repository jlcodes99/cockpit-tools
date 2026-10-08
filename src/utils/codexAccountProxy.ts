import { isCodexApiKeyAccount, isCodexWebSessionAccount, isCodexPendingOAuthAccount, type CodexAccount } from '../types/codex';

export const CODEX_OPEN_PROXY_EVENT = 'codex-open-account-proxy';

/** Proxy eligibility is based on account kind, never subscription tier. */
export function canUseCodexAccountProxy(account: CodexAccount): boolean {
  return !isCodexApiKeyAccount(account)
    && !account.agent_identity
    && !isCodexWebSessionAccount(account)
    && !isCodexPendingOAuthAccount(account)
    && !account.upstream_grok_account_id?.trim()
    && account.api_provider_mode !== 'custom'
    && !account.api_provider_id?.trim();
}

export function requestCodexAccountProxy(accountId: string): void {
  window.dispatchEvent(new CustomEvent(CODEX_OPEN_PROXY_EVENT, { detail: accountId }));
}
