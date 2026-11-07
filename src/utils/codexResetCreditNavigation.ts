// Preserve the selected account when a dashboard shortcut mounts the Codex page.
export const CODEX_RESET_CREDIT_DETAILS_REQUEST = 'codex-reset-credit-details-request';
let pendingAccountId: string | null = null;
export function requestCodexResetCreditDetails(accountId: string) {
  pendingAccountId = accountId;
  window.dispatchEvent(new Event(CODEX_RESET_CREDIT_DETAILS_REQUEST));
}
export function getPendingCodexResetCreditDetails() { return pendingAccountId; }
export function clearPendingCodexResetCreditDetails() { pendingAccountId = null; }
