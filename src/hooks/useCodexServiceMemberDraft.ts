import { useEffect, useId, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import type { CodexAccount } from '../types/codex';
import type { CodexLocalAccessCollection } from '../types/codexLocalAccess';
import { listChatGptPlanAccounts, type ChatGptPlanAccount } from '../services/chatgptPlanService';
import { codexOfficialAuthorizationDeadline } from '../utils/codexOfficialAuthorization';
import { resolveCodexServiceJoinPlan } from '../utils/codexServiceJoin';
import { canChooseCodexServiceAuthorization, requestCodexServiceDraftAuthorizations,
  cancelCodexServiceDraftAuthorizations, CODEX_ACCOUNT_SERVICE_DRAFT_RESULT,
  type CodexAccountServiceDraftResult } from '../utils/codexAccountServiceNavigation';
type Choice = { mode: 'codex' | 'official'; registrationId?: string };
type PendingChoice = { requestId: string; sourceId: string; finish: (accepted: boolean) => void };
/** Membership and access dropdowns edit one draft; only explicit Official starts OAuth. */
export function useCodexServiceMemberDraft(options: {
  active: boolean; accounts: readonly CodexAccount[]; collection: CodexLocalAccessCollection | null;
  grants: readonly ChatGptPlanAccount[]; bindings: Readonly<Record<string, string>>;
  selected: ReadonlySet<string>; setSelected: Dispatch<SetStateAction<Set<string>>>;
  setBindings: Dispatch<SetStateAction<Record<string, string>>>;
  markDirty: () => void; reportError: (error: unknown) => void; clearError?: () => void;
}) {
  const latest = useRef(options); latest.current = options;
  const prefix = useId(), sequence = useRef(0), epoch = useRef(0);
  const pending = useRef<PendingChoice | null>(null), choices = useRef(new Map<string, Choice>());
  const running = useRef<number | null>(null);
  const [busy, setBusy] = useState(false);
  const cancelPending = () => {
    const current = pending.current; pending.current = null;
    if (current) { current.finish(false); cancelCodexServiceDraftAuthorizations(current.requestId); }
  };
  const stage = (id: string, choice: Choice) => {
    choices.current.set(id, choice);
    latest.current.setBindings((previous) => {
      const next = { ...previous };
      if (choice.mode === 'official' && choice.registrationId) next[id] = choice.registrationId;
      else delete next[id];
      return next;
    });
    latest.current.markDirty();
  };
  const choiceFor = (id: string): Choice => {
    const current = latest.current;
    const explicit = choices.current.get(id); if (explicit) return explicit;
    if (current.bindings[id]) return { mode: 'official', registrationId: current.bindings[id] };
    const account = current.accounts.find((row) => row.id === id);
    if (!account || current.collection?.accountIds.includes(id)) return { mode: 'codex' };
    const plan = resolveCodexServiceJoinPlan(account, current.grants);
    return plan.kind === 'saved' ? plan : { mode: 'codex' };
  };
  useEffect(() => {
    const revision = ++epoch.current; running.current = null; setBusy(false);
    if (!options.active) return;
    const receive = (event: Event) => {
      const detail = (event as CustomEvent<CodexAccountServiceDraftResult>).detail;
      const current = pending.current;
      if (!detail || current?.requestId !== detail.requestId || current.sourceId !== detail.sourceAccountId || revision !== epoch.current) return;
      if (detail.status !== 'selected' || detail.mode !== 'official' || !latest.current.accounts.some((account) => account.id === detail.sourceAccountId)) { cancelPending(); return; }
      stage(detail.sourceAccountId, detail);
      pending.current = null; current.finish(true);
    };
    window.addEventListener(CODEX_ACCOUNT_SERVICE_DRAFT_RESULT, receive);
    return () => { epoch.current++; window.removeEventListener(CODEX_ACCOUNT_SERVICE_DRAFT_RESULT, receive); cancelPending(); choices.current.clear(); };
  }, [options.active]);
  const addMembers = async (rows: readonly CodexAccount[]): Promise<string[]> => latest.current.active && running.current !== epoch.current ? rows.map((row) => row.id) : [];
  const chooseMode = async (id: string, mode: Choice['mode']): Promise<void> => {
    if (!latest.current.active || running.current === epoch.current) return;
    const account = latest.current.accounts.find((row) => row.id === id);
    if (!account || !canChooseCodexServiceAuthorization(account)) return;
    latest.current.clearError?.();
    if (mode === 'codex') { stage(id, { mode }); return; }
    const revision = epoch.current; running.current = revision; setBusy(true);
    try {
      const grants = await codexOfficialAuthorizationDeadline(listChatGptPlanAccounts());
      if (revision !== epoch.current || !latest.current.active) return;
      const matches = grants.filter((grant) => grant.source_account_id === id);
      if (matches.length > 1) throw new Error('CHATGPT_PLAN_BINDING_CONFLICT');
      const grant = matches[0];
      if (grant?.connected && !grant.needs_reauth && !grant.source_binding_error) stage(id, { mode, registrationId: grant.id });
      else await new Promise<boolean>((finish) => {
        const requestId = `${prefix}:${++sequence.current}`;
        pending.current = { requestId, sourceId: id, finish };
        requestCodexServiceDraftAuthorizations([id], requestId);
      });
    } catch (error) { if (revision === epoch.current) latest.current.reportError(error); }
    finally { if (revision === epoch.current) { running.current = null; setBusy(false); } }
  };
  const prepareSave = async (ids: readonly string[], _bindings?: Readonly<Record<string, string>>): Promise<Record<string, string> | null> => {
    if (!latest.current.active || running.current === epoch.current) return null;
    // The backend revalidates credential, identity and collection revisions. Saving
    // never opens OAuth or changes the route shown in a row's dropdown.
    return Object.fromEntries(ids.flatMap((id) => {
      const choice = choiceFor(id);
      return choice.mode === 'official' && choice.registrationId ? [[id, choice.registrationId]] : [];
    }));
  };
  return { addMembers, prepareSave, chooseMode, modeFor: (id: string) => choiceFor(id).mode, busy };
}
