import type { CodexAccount } from '../types/codex';
import type { CodexLocalAccessState } from '../types/codexLocalAccess';
import { isChatGptPlanServiceModeApplied, setChatGptPlanServiceMode, type ChatGptPlanAccount } from '../services/chatgptPlanService';
import { chatGptPlanOverviewId } from './chatgptPlanOverview';
import { announceCodexServiceAuthorizationApplied, canChooseCodexServiceAuthorization } from './codexAccountServiceNavigation';

export type CodexServiceJoinPlan =
  | { kind: 'local' }
  | { kind: 'reauthorize' }
  | { kind: 'saved'; mode: 'codex' | 'official'; registrationId: string };

/** Joining never recommends or starts authorization; saved routes are reused explicitly. */
export function resolveCodexServiceJoinPlan(
  account: CodexAccount,
  grants: readonly ChatGptPlanAccount[],
  memberIds: readonly string[] = [],
  officialAccountBindings?: Readonly<Record<string, string>>,
): CodexServiceJoinPlan {
  if (!canChooseCodexServiceAuthorization(account)) return { kind: 'local' };
  const linked = grants.filter((grant) => grant.source_account_id === account.id);
  if (linked.length > 1 || (linked[0] && grants.filter((grant) => grant.id === linked[0].id).length !== 1)) return { kind: 'reauthorize' };
  const grant = linked[0];
  if (officialAccountBindings?.[account.id] && grant?.id !== officialAccountBindings[account.id]) return { kind: 'reauthorize' };
  if (!grant) return { kind: 'local' };
  // Read existing membership first to migrate registrations saved before route
  // preferences existed, including a deliberate choice of the local gateway.
  const officialMember = officialAccountBindings ? officialAccountBindings[account.id] === grant.id && memberIds.includes(account.id) : memberIds.includes(chatGptPlanOverviewId(grant.id));
  const localMember = memberIds.includes(account.id);
  const saved = officialAccountBindings && localMember ? officialMember ? 'official' : 'codex'
    : grant.last_service_mode ?? (officialMember ? 'official' : localMember ? 'codex' : undefined);
  if (saved === 'codex') return { kind: 'saved', mode: 'codex', registrationId: grant.id };
  const usable = grant.connected && !grant.needs_reauth && !grant.source_binding_error;
  if (!usable) return { kind: 'reauthorize' };
  // The backend revalidates the binding and credentials before atomically
  // switching. A pending background identity check is not a reason to re-login.
  return { kind: 'saved', mode: 'official', registrationId: grant.id };
}

export type CodexServiceJoinResult =
  | { kind: 'local' }
  | { kind: 'joined'; state: CodexLocalAccessState; selectedIds: string[] };
export interface CodexServiceJoinFailure { accountId: string; error: unknown }
interface JoinDependencies {
  setMode: typeof setChatGptPlanServiceMode;
  applied: typeof announceCodexServiceAuthorizationApplied;
}
const defaults: JoinDependencies = { setMode: setChatGptPlanServiceMode, applied: announceCodexServiceAuthorizationApplied };

/** Reuse is a real backend transaction, never only a checked frontend row. */
export async function reuseCodexServiceJoin(
  account: CodexAccount,
  grants: readonly ChatGptPlanAccount[],
  memberIds: readonly string[] = [],
  dependencies: JoinDependencies = defaults,
): Promise<CodexServiceJoinResult> {
  const plan = resolveCodexServiceJoinPlan(account, grants, memberIds);
  if (plan.kind === 'reauthorize') throw new Error('CHATGPT_PLAN_REAUTH_REQUIRED');
  if (plan.kind !== 'saved') return plan;
  const state = await dependencies.setMode(account.id, plan.mode, plan.registrationId, { intent: 'join' });
  if (!isChatGptPlanServiceModeApplied(state, account.id, plan.mode, plan.registrationId)) throw new Error('CHATGPT_PLAN_SERVICE_CHANGED');
  const selectedIds = [account.id];
  dependencies.applied([account.id], selectedIds);
  return { kind: 'joined', state, selectedIds };
}

/** Imports use local access when unbound and preserve saved routes without an authorization prompt. */
export async function prepareCodexServiceJoins(
  accounts: readonly CodexAccount[],
  grants: readonly ChatGptPlanAccount[],
  memberIds: readonly string[],
  onJoined: (state: CodexLocalAccessState) => void,
  reuse = reuseCodexServiceJoin,
): Promise<{ localIds: string[]; resumedIds: string[]; failures: CodexServiceJoinFailure[] }> {
  const localIds: string[] = [], resumedIds: string[] = [];
  const failures: CodexServiceJoinFailure[] = [];
  let members = memberIds;
  for (const account of accounts) {
    let result: CodexServiceJoinResult;
    try { result = await reuse(account, grants, members); }
    catch (error) {
      // Preserve the saved route and keep processing unrelated accounts. The
      // caller reports every failure after committing the successful local IDs.
      failures.push({ accountId: account.id, error });
      continue;
    }
    if (result.kind === 'joined') {
      members = result.state.collection?.accountIds ?? members;
      resumedIds.push(...result.selectedIds);
      onJoined(result.state);
    } else if (result.kind === 'local') localIds.push(account.id);
  }
  return { localIds, resumedIds, failures };
}
