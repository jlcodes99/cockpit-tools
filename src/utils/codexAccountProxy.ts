import { isCodexApiKeyAccount, isCodexWebSessionAccount, isCodexPendingOAuthAccount, type CodexAccount } from '../types/codex';

export const CODEX_OPEN_PROXY_EVENT = 'codex-open-account-proxy';

/** Existing OAuth request/auth eligibility, independent of the card's desktop proxy settings. */
export function canUseCodexOAuthAccountProxy(account: CodexAccount): boolean {
  return !isCodexApiKeyAccount(account)
    && !account.agent_identity
    && !isCodexWebSessionAccount(account)
    && !isCodexPendingOAuthAccount(account)
    && !account.upstream_grok_account_id?.trim()
    && account.api_provider_mode !== 'custom'
    && !account.api_provider_id?.trim();
}

/** Cards expose the same proxy interaction for OAuth accounts, API accounts and the API service. */
export function canUseCodexAccountProxy(account: CodexAccount): boolean {
  return isCodexApiKeyAccount(account) || canUseCodexOAuthAccountProxy(account);
}

export function requestCodexAccountProxy(accountId: string): void {
  window.dispatchEvent(new CustomEvent(CODEX_OPEN_PROXY_EVENT, { detail: accountId }));
}
