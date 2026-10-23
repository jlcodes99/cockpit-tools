import type { CodexAccount } from '../types/codex';
import { isChatGptPlanAuthorizationUsable, type ChatGptPlanAccount } from '../services/chatgptPlanService';
import { canChooseCodexServiceAuthorization } from './codexAccountServiceNavigation';
import type { CodexServiceMemberAccess } from '../types/codexLocalAccess';

export interface CodexOfficialAuthorizationRow {
  account: CodexAccount;
  grant?: ChatGptPlanAccount;
  joined: boolean;
  official: boolean;
  authorized: boolean;
  paused: boolean;
}

export function codexOfficialAuthorizationRows(accounts: readonly CodexAccount[], grants: readonly ChatGptPlanAccount[], members: readonly string[], pausedMembers: readonly string[] = [], serviceMembers?: readonly CodexServiceMemberAccess[]): CodexOfficialAuthorizationRow[] {
  const ids = new Set(members);
  const paused = new Set(pausedMembers);
  return accounts.filter(canChooseCodexServiceAuthorization).map((account) => {
    const linked = grants.filter((grant) => grant.source_account_id === account.id);
    const grant = linked.length === 1 ? linked[0] : undefined;
    const access = serviceMembers?.find((member) => member.sourceAccountId === account.id && ids.has(member.accountId));
    const official = serviceMembers ? access?.mode === 'official' : Boolean(grant && ids.has(`chatgpt_plan_${grant.id}`));
    return { account, grant, official, joined: Boolean(access || ids.has(account.id) || official),
      paused: access?.paused ?? Boolean((ids.has(account.id) && paused.has(account.id)) || (official && grant && paused.has(`chatgpt_plan_${grant.id}`))),
      authorized: isChatGptPlanAuthorizationUsable(grant) };
  }).sort((a, b) => Number(b.joined) - Number(a.joined));
}

/** Automatic reminders are scoped to the persisted API Service member pool. */
export function needsCodexOfficialAuthorizationReview(row: CodexOfficialAuthorizationRow): boolean {
  return row.joined && !row.paused && (!row.grant || !row.grant.connected || row.grant.needs_reauth || Boolean(row.grant.source_binding_error));
}

export function codexOfficialAuthorizationRelatedIds(sourceId: string, grants: readonly ChatGptPlanAccount[]): string[] {
  return [sourceId, ...grants.filter((grant) => grant.source_account_id === sourceId).map((grant) => `chatgpt_plan_${grant.id}`)];
}

export type CodexOfficialAuthorizationState = 'used' | 'unused' | 'reauthorize' | 'disconnected' | 'pending' | 'invalid';
export function codexOfficialAuthorizationState(grant: ChatGptPlanAccount | undefined, active: boolean): CodexOfficialAuthorizationState {
  if (!grant) return 'disconnected';
  if (grant.needs_reauth) return 'reauthorize';
  if (!grant.connected) return 'disconnected';
  if (grant.source_binding_error) return 'invalid';
  if (grant.source_account_id && grant.source_binding_valid !== true) return 'pending';
  return active ? 'used' : 'unused';
}

export function codexOfficialAuthorizationStateKey(state: CodexOfficialAuthorizationState): string {
  return ({ used: 'chatgptPlan.officialFlow.authorizedInUse', unused: 'chatgptPlan.officialFlow.authorizedUnused',
    reauthorize: 'chatgptPlan.status.reauthorize', disconnected: 'chatgptPlan.officialFlow.notAuthorized',
    pending: 'chatgptPlan.officialFlow.bindingChecking', invalid: 'chatgptPlan.officialFlow.bindingInvalid' })[state];
}

/** Timeout never cancels or retries a mutation. Callers still fence late UI results. */
export async function codexOfficialAuthorizationDeadline<T>(operation: Promise<T>, ms = 15_000): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([operation, new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('CHATGPT_PLAN_TIMEOUT')), ms);
    })]);
  } finally { if (timer) clearTimeout(timer); }
}
