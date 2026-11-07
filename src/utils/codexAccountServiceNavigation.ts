import { isCodexApiKeyAccount, isCodexAgentIdentityAccount, isCodexWebSessionAccount, isCodexPendingOAuthAccount, type CodexAccount } from '../types/codex';

export const CODEX_ACCOUNT_SERVICE_REQUEST = 'codex-account-service-request';
export const CODEX_ACCOUNT_SERVICE_APPLIED = 'codex-account-service-applied';
export const CODEX_ACCOUNT_SERVICE_DRAFT_RESULT = 'codex-account-service-draft-result';
export const CODEX_ACCOUNT_SERVICE_DRAFT_CANCEL = 'codex-account-service-draft-cancel';
export const CODEX_OFFICIAL_AUTHORIZATION_MANAGE = 'codex-official-authorization-manage';
let pendingManage = false;
export function requestCodexOfficialAuthorizationManagement(): void {
  pendingManage = true;
  window.dispatchEvent(new Event(CODEX_OFFICIAL_AUTHORIZATION_MANAGE));
}
export function takeCodexOfficialAuthorizationManagement(): boolean {
  const next = pendingManage; pendingManage = false; return next;
}
export interface CodexAccountServiceApplied { relatedIds: string[]; selectedIds: string[] }
export function announceCodexServiceAuthorizationApplied(relatedIds: string[], selectedIds: string[]): void {
  window.dispatchEvent(new CustomEvent<CodexAccountServiceApplied>(CODEX_ACCOUNT_SERVICE_APPLIED, { detail: { relatedIds, selectedIds } }));
}
export interface CodexAccountServiceRequest { accountId: string; initialMode?: 'codex' | 'official'; draftRequestId?: string }
export type CodexAccountServiceDraftResult = { requestId: string; sourceAccountId: string } & (
  | { status: 'selected'; mode: 'codex'; registrationId?: never }
  | { status: 'selected'; mode: 'official'; registrationId: string }
  | { status: 'cancelled' }
);
const draftRequests = new Map<string, Set<string>>();
let draftSequence = 0;
let pending: CodexAccountServiceRequest[] = [];
/** A card link never changes the underlying credential kind. */
export function canChooseCodexServiceAuthorization(account: CodexAccount): boolean {
  return !isCodexApiKeyAccount(account) && !isCodexAgentIdentityAccount(account)
    && !isCodexWebSessionAccount(account) && !isCodexPendingOAuthAccount(account)
    && !account.upstream_grok_account_id && !account.id.startsWith('codex_grok_')
    && (!account.api_provider_mode || account.api_provider_mode === 'openai_builtin')
    && !account.api_provider_id && !account.openai_api_key;
}

/** A batch is one review dialog, not a sequence of modal interruptions. */
export function requestCodexServiceAuthorizations(accountIds: readonly string[]): void {
  for (const value of accountIds) {
    const accountId = value.trim();
    if (accountId && !pending.some((item) => item.accountId === accountId && !item.draftRequestId)) pending.push({ accountId });
  }
  if (pending.length) window.dispatchEvent(new Event(CODEX_ACCOUNT_SERVICE_REQUEST));
}

export function requestCodexServiceAuthorization(accountId: string, initialMode?: 'codex' | 'official'): void {
  if (!accountId.trim()) return;
  if (!pending.some((item) => item.accountId === accountId && !item.draftRequestId)) pending.push({ accountId, initialMode });
  window.dispatchEvent(new Event(CODEX_ACCOUNT_SERVICE_REQUEST));
}

/** A draft request can save authorization, but never commits service membership. */
export function requestCodexServiceDraftAuthorizations(accountIds: readonly string[], requestId = `codex-draft-${Date.now()}-${++draftSequence}`): string {
  const ids = [...new Set(accountIds.map((id) => id.trim()).filter(Boolean))];
  if (!ids.length) return requestId;
  const outstanding = draftRequests.get(requestId) ?? new Set<string>();
  for (const accountId of ids) {
    if (outstanding.has(accountId)) continue;
    outstanding.add(accountId);
    pending.push({ accountId, draftRequestId: requestId });
  }
  draftRequests.set(requestId, outstanding);
  window.dispatchEvent(new Event(CODEX_ACCOUNT_SERVICE_REQUEST));
  return requestId;
}

export function isCodexServiceDraftRequestActive(requestId: string, sourceAccountId?: string): boolean {
  const request = draftRequests.get(requestId);
  return Boolean(request && (sourceAccountId === undefined || request.has(sourceAccountId)));
}

/** Delivery is at most once and cannot revive a cancelled editor. */
export function announceCodexServiceDraftResult(result: CodexAccountServiceDraftResult): boolean {
  const outstanding = draftRequests.get(result.requestId);
  if (!outstanding?.has(result.sourceAccountId)) return false;
  if (result.status === 'selected' && result.mode === 'official' && !result.registrationId?.trim()) return false;
  outstanding.delete(result.sourceAccountId);
  if (!outstanding.size) draftRequests.delete(result.requestId);
  window.dispatchEvent(new CustomEvent<CodexAccountServiceDraftResult>(CODEX_ACCOUNT_SERVICE_DRAFT_RESULT, { detail: result }));
  return true;
}

export function cancelCodexServiceDraftAuthorizations(requestId: string): void {
  const outstanding = draftRequests.get(requestId);
  draftRequests.delete(requestId);
  pending = pending.filter((item) => item.draftRequestId !== requestId);
  window.dispatchEvent(new CustomEvent(CODEX_ACCOUNT_SERVICE_DRAFT_CANCEL, { detail: { requestId } }));
  for (const sourceAccountId of outstanding ?? []) {
    window.dispatchEvent(new CustomEvent<CodexAccountServiceDraftResult>(CODEX_ACCOUNT_SERVICE_DRAFT_RESULT, { detail: { requestId, sourceAccountId, status: 'cancelled' } }));
  }
}

/** Requeue with the original intent; never turn a deferred draft into a join. */
export function requeueCodexServiceAuthorizationRequests(items: readonly CodexAccountServiceRequest[]): void {
  for (const item of items) {
    if (item.draftRequestId && !isCodexServiceDraftRequestActive(item.draftRequestId, item.accountId)) continue;
    if (!pending.some((saved) => saved.accountId === item.accountId && saved.draftRequestId === item.draftRequestId)) pending.push(item);
  }
  if (pending.length) window.dispatchEvent(new Event(CODEX_ACCOUNT_SERVICE_REQUEST));
}

export function takeCodexServiceAuthorizationRequests(): CodexAccountServiceRequest[] {
  const next = pending;
  pending = [];
  return next;
}

/** Rebase persisted membership changes without discarding unrelated selection edits. */
export function rebaseCodexMemberSelection(draft: ReadonlySet<string>, before: readonly string[], after: readonly string[]): Set<string> {
  const next = new Set(draft);
  const previous = new Set(before);
  const current = new Set(after);
  for (const id of previous) if (!current.has(id)) next.delete(id);
  for (const id of current) if (!previous.has(id)) next.add(id);
  return next;
}
