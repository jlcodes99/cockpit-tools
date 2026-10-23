import { loadHookModule } from './reactHookHarness';
import { resolveCodexServiceJoinPlan } from '../../src/utils/codexServiceJoin';
import { canChooseCodexServiceAuthorization } from '../../src/utils/codexAccountServiceNavigation';
import type { CodexAccount } from '../../src/types/codex';

/** Runs the real production hook; only I/O and its parent's draft setters are replaced. */
export function createCodexServiceMemberDraftHarness(options: {
  accounts: CodexAccount[];
  list?: () => Promise<any[]>;
  grants?: any[];
  collection?: { accountIds: string[]; officialAccountBindings: Record<string, string> };
}) {
  const window = new EventTarget();
  const calls = { lists: 0, mutations: 0, choices: [] as { ids: string[]; requestId: string }[], cancelled: [] as string[], errors: [] as unknown[] };
  let active = true, dirty = false;
  const collection = options.collection ?? { accountIds: [], officialAccountBindings: {} };
  let selected = new Set<string>(collection.accountIds);
  let bindings: Record<string, string> = { ...collection.officialAccountBindings };
  const setSelected = (update: Set<string> | ((previous: Set<string>) => Set<string>)) => {
    selected = typeof update === 'function' ? update(selected) : update;
  };
  const hook = loadHookModule(new URL('../../src/hooks/useCodexServiceMemberDraft.ts', import.meta.url), {
    '../services/chatgptPlanService': {
      listChatGptPlanAccounts: async () => { calls.lists++; return options.list?.() ?? []; },
      setChatGptPlanServiceMode: async () => { calls.mutations++; throw new Error('draft must not mutate service membership'); },
    },
    '../utils/codexOfficialAuthorization': { codexOfficialAuthorizationDeadline: (promise: unknown) => promise },
    '../utils/codexServiceJoin': {
      resolveCodexServiceJoinPlan: (account: CodexAccount, grants: any[], members: string[], saved?: Record<string, string>) =>
        resolveCodexServiceJoinPlan(account, grants, members, saved),
      reuseCodexServiceJoin: async () => { calls.mutations++; throw new Error('draft must not reuse an immediate join'); },
    },
    '../utils/codexAccountServiceNavigation': {
      canChooseCodexServiceAuthorization,
      CODEX_ACCOUNT_SERVICE_DRAFT_RESULT: 'draft-result',
      requestCodexServiceDraftAuthorizations: (ids: string[], requestId: string) => { calls.choices.push({ ids: [...ids], requestId }); return requestId; },
      cancelCodexServiceDraftAuthorizations: (id: string) => calls.cancelled.push(id),
    },
  }, { window });
  hook.render(() => hook.exports.useCodexServiceMemberDraft({
    active, accounts: options.accounts, collection, selected, setSelected, grants: options.grants ?? [], bindings,
    setBindings: (update: (previous: Record<string, string>) => Record<string, string>) => { bindings = update(bindings); },
    markDirty: () => { dirty = true; }, reportError: (value: unknown) => { calls.errors.push(value); },
  }));
  const addRows = async (rows = options.accounts): Promise<string[]> => Array.from(await hook.flush().addMembers(rows));
  return {
    hook, calls, collection, addRows, setSelected, selected: () => selected,
    prepare: async (): Promise<Record<string, string> | null> => {
      const result = await hook.flush().prepareSave([...selected], bindings);
      return result === null ? null : JSON.parse(JSON.stringify(result));
    },
    choose: async (mode: 'codex' | 'official', id = options.accounts[0].id) => { const result = hook.flush().chooseMode(id, mode); hook.flush(); await result; hook.flush(); },
    mode: (id = options.accounts[0].id): string => hook.flush().modeFor(id),
    bindings: (): Record<string, string> => JSON.parse(JSON.stringify(bindings)), dirty: () => dirty,
    error: () => calls.errors[calls.errors.length - 1],
    open: (value: boolean) => { active = value; hook.flush(); },
    add: async (rows = options.accounts) => { const ids = await addRows(rows); selected = new Set([...selected, ...ids]); hook.flush(); return ids; },
    emit: (detail: unknown) => { window.dispatchEvent(new CustomEvent('draft-result', { detail })); hook.flush(); },
  };
}
