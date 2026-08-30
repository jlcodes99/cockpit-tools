export const CODEX_OPEN_ADD_ACCOUNT_EVENT = 'codex-open-add-account';
export const CODEX_SUITE_ENSURE_MOUNTED_EVENT = 'codex-suite-ensure-mounted';

export type CodexAddAccountTab = 'oauth' | 'token' | 'apikey' | 'import';

/** OAuth 重新授权成功后，继续完成原绑定目标所需的上下文。 */
export type CodexOAuthBindingRetryDetail = {
  targetKind: 'local_access' | 'api_key_account';
  targetAccountId?: string;
  quotaReserve?: { hourlyPercent: number; weeklyPercent: number } | null;
};

export type CodexOpenAddAccountDetail = {
  autoJoinApiService?: boolean;
  targetAccountId?: string;
  retrySwitchAfterOAuth?: boolean;
  retrySwitchLaunchAfterSwitch?: boolean;
  retryInstanceLaunchAfterOAuth?: boolean;
  retryInstanceId?: string;
  retryOAuthBinding?: CodexOAuthBindingRetryDetail;
  tab?: CodexAddAccountTab;
};

let pendingOpenRequest: CodexOpenAddAccountDetail | null = null;

export function takePendingCodexOpenAddAccountRequest(): CodexOpenAddAccountDetail | null {
  const request = pendingOpenRequest;
  pendingOpenRequest = null;
  return request;
}

/** Ask Codex suite pages to stay mounted and open the shared add-account modal. */
export function requestCodexOpenAddAccount(detail: CodexOpenAddAccountDetail = {}): void {
  const autoJoinApiService = detail.autoJoinApiService === true;
  const targetAccountId = detail.targetAccountId?.trim() || undefined;
  const retrySwitchAfterOAuth = detail.retrySwitchAfterOAuth === true;
  const retrySwitchLaunchAfterSwitch =
    typeof detail.retrySwitchLaunchAfterSwitch === 'boolean'
      ? detail.retrySwitchLaunchAfterSwitch
      : undefined;
  const retryInstanceLaunchAfterOAuth = detail.retryInstanceLaunchAfterOAuth === true;
  const retryInstanceId = detail.retryInstanceId?.trim() || undefined;
  const retryOAuthBinding = detail.retryOAuthBinding
    ? {
        targetKind: detail.retryOAuthBinding.targetKind,
        targetAccountId: detail.retryOAuthBinding.targetAccountId?.trim() || undefined,
        quotaReserve: detail.retryOAuthBinding.quotaReserve ?? null,
      }
    : undefined;
  const tab = detail.tab ?? 'oauth';
  const normalized = {
    autoJoinApiService,
    targetAccountId,
    retrySwitchAfterOAuth,
    retrySwitchLaunchAfterSwitch,
    retryInstanceLaunchAfterOAuth,
    retryInstanceId,
    retryOAuthBinding,
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
