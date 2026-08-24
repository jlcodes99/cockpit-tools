export const CODEX_OPEN_ADD_ACCOUNT_EVENT = 'codex-open-add-account';
export const CODEX_SUITE_ENSURE_MOUNTED_EVENT = 'codex-suite-ensure-mounted';

export type CodexAddAccountTab = 'oauth' | 'token' | 'apikey' | 'import';

export type CodexOpenAddAccountDetail = {
  autoJoinApiService?: boolean;
  targetAccountId?: string;
  retrySwitchAfterOAuth?: boolean;
  retryInstanceLaunchAfterOAuth?: boolean;
  retryInstanceId?: string;
  tab?: CodexAddAccountTab;
};

let pendingOpenRequest: CodexOpenAddAccountDetail | null = null;

export function takePendingCodexOpenAddAccountRequest(): CodexOpenAddAccountDetail | null {
  const request = pendingOpenRequest;
  pendingOpenRequest = null;
  return request;
}

/** Ask Codex suite pages to stay mounted and open the shared add-account modal. */
export function requestCodexOpenAddAccount(
  detail: CodexOpenAddAccountDetail = {},
): void {
  const autoJoinApiService = detail.autoJoinApiService === true;
  const targetAccountId = detail.targetAccountId?.trim() || undefined;
  const retrySwitchAfterOAuth = detail.retrySwitchAfterOAuth === true;
  const retryInstanceLaunchAfterOAuth =
    detail.retryInstanceLaunchAfterOAuth === true;
  const retryInstanceId = detail.retryInstanceId?.trim() || undefined;
  const tab = detail.tab ?? 'oauth';
  const normalized = {
    autoJoinApiService,
    targetAccountId,
    retrySwitchAfterOAuth,
    retryInstanceLaunchAfterOAuth,
    retryInstanceId,
    tab,
  } satisfies CodexOpenAddAccountDetail;
  pendingOpenRequest = normalized;
  window.dispatchEvent(new CustomEvent(CODEX_SUITE_ENSURE_MOUNTED_EVENT));
  window.dispatchEvent(
    new CustomEvent(CODEX_OPEN_ADD_ACCOUNT_EVENT, {
      detail: normalized,
    }),
  );
}
