import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { codexServiceRelatedEndpointIds, codexServiceMemberAccounts, projectCodexServiceAccountDisplays } from '../utils/chatgptPlanOverview';
import { buildCodexServiceMemberView, filterCodexServiceMemberView, needsCodexServiceMemberAuthorizationChoice } from "../utils/codexServiceMemberView";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import { VALID_ACCOUNTS_FILTER_VALUE } from "../utils/accountValidityFilter";
import { resolveCodexLocalAccessInitialAccountIds, resolveCodexMemberImagePolicy } from '../utils/codexLocalAccessAccounts';
import type { CodexAccount } from '../types/codex';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';
import { chatGptPlanErrorKey } from '../services/chatgptPlanService';
import { createCodexServiceMemberDraftHarness } from '../../tests/helpers/codexServiceMemberDraftHarness';
import { deferred, settlePromises } from '../../tests/helpers/reactHookHarness';

function subscription(open = true, mode = 'members') {
  const source = ts.createSourceFile('modal.tsx', readFileSync(new URL('./CodexLocalAccessModal.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let callback: ts.Node | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node) && node.expression.getText(source) === 'useEffect'
      && node.arguments[0]?.getText(source).includes('CODEX_ACCOUNT_SERVICE_APPLIED')) callback = node.arguments[0];
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(callback);
  const window = new EventTarget();
  const draft = {
    selected: new Set(['source', 'unrelated-pending']),
    policies: { source: 'enabled', 'unrelated-pending': 'disabled' } as Record<string, string>,
    routing: { source: { priority: 9, isBackup: true }, 'unrelated-pending': { priority: 3 } } as Record<string, unknown>,
    dirty: false,
  };
  const expression = ts.transpileModule(`(${callback.getText(source)})`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const run = vm.runInNewContext(expression, {
    window, isOpen: open, mode, CODEX_ACCOUNT_SERVICE_APPLIED: 'applied',
    setMembersDraftDirty: (value: boolean) => { draft.dirty = value; },
    setSelected: (update: (value: Set<string>) => Set<string>) => { draft.selected = update(draft.selected); },
    setImageGenerationPolicies: (update: (value: Record<string, string>) => Record<string, string>) => { draft.policies = update(draft.policies); },
    setCustomRoutingDraft: (update: (value: Record<string, unknown>) => Record<string, unknown>) => { draft.routing = update(draft.routing); },
  });
  const cleanup = run();
  const dispatch = (selectedIds: string[]) => window.dispatchEvent(new CustomEvent('applied', { detail: { relatedIds: ['source', 'chatgpt_plan_grant'], selectedIds } }));
  return { draft, dispatch, cleanup };
}

test('official service application rebases only its own members and preserves unrelated edits', () => {
  const h = subscription();
  h.dispatch(['chatgpt_plan_grant']);
  assert.deepEqual([...h.draft.selected], ['unrelated-pending', 'chatgpt_plan_grant']);
  assert.equal(h.draft.policies.chatgpt_plan_grant, 'disabled');
  assert.equal(h.draft.policies['unrelated-pending'], 'disabled');
  assert.equal(h.draft.policies.source, undefined);
  assert.deepEqual(h.draft.routing.chatgpt_plan_grant, { priority: 9, isBackup: true });
  assert.equal(h.draft.dirty, true);
  h.cleanup();
  h.dispatch(['source']);
  assert.ok(h.draft.selected.has('chatgpt_plan_grant'));
});

test('local gateway application selects its source and inactive member dialogs do not intercept updates', () => {
  const h = subscription();
  h.dispatch(['source']);
  assert.deepEqual([...h.draft.selected], ['unrelated-pending', 'source']);
  assert.equal(h.draft.policies.source, 'enabled');
  h.cleanup();
  for (const item of [subscription(false), subscription(true, 'settings')]) {
    item.dispatch(['chatgpt_plan_grant']);
    assert.deepEqual([...item.draft.selected], ['source', 'unrelated-pending']);
    assert.equal(item.draft.dirty, false);
  }
});

function saveMembersContext(options: { mappingEnabled?: boolean; selected?: Set<string>; accounts?: Map<string, { id: string }>; bindings?: Record<string, string>; save?: (payload: any) => Promise<unknown>; prepare?: (ids: string[], bindings: Record<string, string>) => Promise<Record<string, string> | null> } = {}) {
  const source = ts.createSourceFile('modal.tsx', readFileSync(new URL('./CodexLocalAccessModal.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let callback: ts.Expression | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'handleSaveMembers') callback = node.initializer;
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(callback);
  const selected = options.selected ?? new Set(['chatgpt_plan_grant', 'other']);
  const accounts = options.accounts ?? new Map([['other', { id: 'other' }]]);
  let error = '';
  let closed = false;
  let pending = false;
  const memberSaveInFlight = { current: false }, memberSaveEpoch = { current: 0 };
  const saved: any[] = [];
  const collection = { accountIds: ['chatgpt_plan_grant', 'other'], pausedAccountIds: [] as string[] };
  const expression = ts.transpileModule(`(${callback.getText(source)})`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const action = vm.runInNewContext(expression, {
    accountsLoaded: true, selected, localAccessAccountById: accounts, collection,
    memberSaveInFlight, memberSaveEpoch, setMemberSavePending: (value: boolean) => { pending = value; },
    prepareMemberSave: options.prepare ?? (async (_ids: string[], bindings: Record<string, string>) => bindings),
    sessionAffinityTtlSeconds: '3600', sessionAffinity: true, restrictFreeAccounts: true,
    sessionIdentityMappingEnabled: options.mappingEnabled ?? true,
    customRoutingDraft: {}, customRoutingRuleByAccountId: new Map(), imageGenerationPolicies: {}, officialAccountBindings: options.bindings ?? {},
    setError: (value: string) => { error = value; }, setNotice() {}, setSessionAffinityTtlError() {},
    t: (key: string) => key, chatGptPlanErrorKey,
    isCodexLocalAccessEligibleAccount: () => true,
    resolveCodexMemberImagePolicy: (account: { id: string }) => account.id.startsWith('chatgpt_plan_') ? 'disabled' : 'inherit',
    onSaveAccounts: async (value: unknown) => { saved.push(value); await options.save?.(value); },
    onClose: () => { closed = true; },
  });
  return { action, selected, accounts, saved, error: () => error, closed: () => closed, pending: () => pending,
    reopen: () => { memberSaveEpoch.current++; memberSaveInFlight.current = false; pending = false; },
  };
}

test('saving waits for official endpoint metadata instead of dropping persisted authorization membership', async () => {
  const h = saveMembersContext();
  await h.action();
  assert.equal(h.error(), 'chatgptPlan.officialFlow.memberSyncPending');
  assert.equal(h.closed(), false);
  assert.equal(h.saved.length, 0);
  assert.deepEqual([...h.selected], ['chatgpt_plan_grant', 'other']);
  h.accounts.set('chatgpt_plan_grant', { id: 'chatgpt_plan_grant' });
  await h.action();
  assert.equal(h.error(), '');
  assert.equal(h.closed(), true);
  assert.deepEqual(Array.from(h.saved[0].accountIds), ['chatgpt_plan_grant', 'other']);
  assert.equal(h.saved[0].imageGenerationAccountPolicies.chatgpt_plan_grant, 'disabled');
});


test('the production picker rebuilds source filters instead of losing healthy official rows to source OAuth failures', () => {
  const file = ts.createSourceFile('modal.tsx', readFileSync(new URL('./CodexLocalAccessModal.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let callback: ts.Expression | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(file) === 'visibleAccounts' && node.initializer && ts.isCallExpression(node.initializer)) callback = node.initializer.arguments[0];
    ts.forEachChild(node, visit);
  };
  visit(file);
  assert.ok(callback);
  const source = { id: 'source', email: 'user@example.com', account_name: 'Original account', plan_type: 'TEAM', requires_reauth: true, tokens: { access_token: 'original-token', id_token: '' } } as CodexAccount;
  const endpoint = { id: 'chatgpt_plan_grant', email: source.email, auth_mode: 'apikey', api_provider_id: 'cockpit_chatgpt_plan', tokens: { access_token: '', id_token: '' } } as CodexAccount;
  const grant = { id: 'grant', source_account_id: source.id, source_binding_valid: true, connected: true, needs_reauth: false } as ChatGptPlanAccount;
  const localAccessAccounts = codexServiceMemberAccounts([source, endpoint], [grant], [endpoint.id]);
  const code = ts.transpileModule(`(${callback.getText(file)})`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const serviceMemberView = buildCodexServiceMemberView({ accounts: localAccessAccounts, sourceAccounts: [source, endpoint],
    groups: [{ id: 'work', name: 'Work', accountIds: [source.id] } as any], memberIds: [endpoint.id] });
  const run = vm.runInNewContext(code, { memberView: { accounts: [], activeGroupId: 'work', compareAccounts: () => 0 },
    serviceMemberView, filterCodexServiceMemberView, buildCodexAccountPresentation, t: (key: string) => key,
    memberSearchQuery: '', memberFilterTypes: ['TEAM', VALID_ACCOUNTS_FILTER_VALUE], memberTagFilter: [], memberGroupFilter: ['work'] });
  const rows = run();
  assert.equal(rows.length, 1);
  assert.equal(rows[0].account_name, 'Original account');
  assert.equal(rows[0].id, endpoint.id);
  const selected = resolveCodexLocalAccessInitialAccountIds([endpoint.id], localAccessAccounts, true, true);
  assert.equal(selected.includes(rows[0].id), true);
  assert.equal(resolveCodexMemberImagePolicy(rows[0], 'enabled'), 'disabled');
  const h = saveMembersContext();
  h.accounts.set(endpoint.id, rows[0]);
  return h.action().then(() => {
    assert.equal(h.saved.length, 1);
    assert.deepEqual(Array.from(h.saved[0].accountIds), [endpoint.id, 'other']);
    assert.equal(h.saved[0].accountIds.includes(source.id), false, 'saving cannot silently re-enable the local gateway');
  });
});

function productionAction(path: string, name: string, context: Record<string, unknown>) {
  const file = ts.createSourceFile('page.tsx', readFileSync(new URL(path, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let callback: ts.Expression | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(file) === name) callback = node.initializer;
    ts.forEachChild(node, visit);
  };
  visit(file);
  assert.ok(callback);
  const code = ts.transpileModule(`(${callback.getText(file)})`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  return vm.runInNewContext(code, context);
}

test('production member selection unchecks both legacy channels without losing unrelated draft members', async () => {
  const { codexServiceRelatedEndpointIds, toggleCodexServiceDisplaySelection } = await import('../utils/chatgptPlanOverview');
  const source = { id: 'source', email: 'a@example.com' } as CodexAccount;
  const official = { id: 'chatgpt_plan_grant', api_provider_id: 'cockpit_chatgpt_plan', auth_mode: 'apikey' } as CodexAccount;
  const grant = { id: 'grant', source_account_id: source.id, source_binding_valid: true } as ChatGptPlanAccount;
  const accounts = codexServiceMemberAccounts([source, official], [grant], [source.id, official.id]);
  const row = accounts.find((account) => account.id === official.id)!;
  const context: Record<string, any> = {
    membersInteractionDisabled: false, localAccessAccounts: accounts, completeLocalAccessAccounts: accounts,
    localAccessAccountById: new Map(accounts.map((account) => [account.id, account])),
    selected: new Set([source.id, official.id, 'unrelated']),
    collection: { accountIds: [source.id, official.id] }, restrictFreeAccounts: false,
    isCodexLocalAccessEligibleAccount: () => true,
    isMemberRowSelected: (account: CodexAccount) => codexServiceRelatedEndpointIds(account, accounts).some((id) => context.selected.has(id)),
    toggleCodexServiceDisplaySelection, needsCodexServiceMemberAuthorizationChoice, setMembersDraftDirty() {},
    addMemberRows: async (rows: CodexAccount[]) => rows.map((account) => account.id),
    setSelected: (update: (selected: Set<string>) => Set<string>) => { context.selected = update(context.selected); },
  };
  const toggle = productionAction('./CodexLocalAccessModal.tsx', 'toggleSelect', context);
  await toggle(row.id);
  assert.deepEqual([...context.selected], ['unrelated']);
  await toggle(row.id);
  assert.deepEqual([...context.selected], ['unrelated', official.id]);
});

test('production API member removal removes both channels and preserves unrelated routing flags', async () => {
  const { codexServiceRelatedEndpointIds } = await import('../utils/chatgptPlanOverview');
  const source = { id: 'source', email: 'a@example.com' } as CodexAccount;
  const official = { id: 'chatgpt_plan_grant', api_provider_id: 'cockpit_chatgpt_plan', auth_mode: 'apikey' } as CodexAccount;
  const grant = { id: 'grant', source_account_id: source.id, source_binding_valid: true } as ChatGptPlanAccount;
  const accounts = codexServiceMemberAccounts([source, official], [grant], [source.id, official.id]);
  const saved: any[] = [];
  const remove = productionAction('../pages/CodexApiServicePage.tsx', 'handleRemoveMember', {
    collection: { accountIds: [source.id, official.id, 'other'], restrictFreeAccounts: true,
      customRoutingRules: [{ accountId: source.id, isBackup: true }, { accountId: 'other', isBackup: true, isPreferred: true }] },
    localAccessAccounts: accounts, codexServiceRelatedEndpointIds,
    handleSaveMembers: (...args: any[]) => { saved.push(args); },
  });
  await remove(official.id);
  assert.deepEqual(Array.from(saved[0][0]), ['other']);
  assert.equal(saved[0][1], true);
  assert.deepEqual(Array.from(saved[0][2]), ['other']);
  assert.deepEqual(Array.from(saved[0][3]), ['other']);
});

test('priority changes select the original member without opening authorization before Save', async () => {
  const source = { id: 'source', email: 'source@example.com', auth_mode: 'oauth' } as CodexAccount;
  const accounts = [source];
  const draft = createCodexServiceMemberDraftHarness({ accounts, list: async () => [] });
  const context: Record<string, any> = {
    membersInteractionDisabled: false, completeLocalAccessAccounts: accounts,
    localAccessAccountById: new Map(accounts.map((account) => [account.id, account])),
    get selected() { return draft.selected(); }, collection: draft.collection, restrictFreeAccounts: false,
    customRoutingDraft: {}, customRoutingRuleByAccountId: new Map(),
    CUSTOM_ROUTING_PRIORITY_MIN: 0, CUSTOM_ROUTING_WEIGHT_MIN: 1,
    isCodexLocalAccessEligibleAccount: () => true,
    isMemberRowSelected: (account: CodexAccount) => codexServiceRelatedEndpointIds(account, accounts).some((id) => draft.selected().has(id)),
    codexServiceRelatedEndpointIds, setMembersDraftDirty() {},
    addMemberRows: draft.addRows, setSelected: draft.setSelected,
    setCustomRoutingDraft: (update: (previous: object) => object) => { context.customRoutingDraft = update(context.customRoutingDraft); },
  };
  const priority = productionAction('./CodexLocalAccessModal.tsx', 'updateMemberUsagePriority', context);
  await priority(source.id, 'highest');
  assert.deepEqual([...draft.selected()], [source.id]);
  assert.equal(context.customRoutingDraft[source.id].isPreferred, true);
  assert.equal(context.customRoutingDraft[source.id].isBackup, false);
  assert.equal(draft.calls.choices.length, 0);
  assert.equal(draft.calls.lists, 0);
  assert.equal(draft.calls.mutations, 0);
  draft.hook.unmount();
});

test('production selection registry retains hidden endpoints during an authorization mode refresh', () => {
  const source = { id: 'source', email: 'source@example.com' } as CodexAccount;
  const endpoint = { id: 'chatgpt_plan_grant', auth_mode: 'apikey', api_provider_id: 'cockpit_chatgpt_plan' } as CodexAccount;
  const grant = { id: 'grant', source_account_id: source.id, source_binding_valid: true } as ChatGptPlanAccount;
  const complete = projectCodexServiceAccountDisplays([source, endpoint], [grant]);
  const visible = codexServiceMemberAccounts([source, endpoint], [grant], [endpoint.id]);
  assert.deepEqual(visible.map((row) => row.id), [endpoint.id]);
  const registry = productionAction('./CodexLocalAccessModal.tsx', 'localAccessAccountById', {
    completeLocalAccessAccounts: complete, useMemo: (factory: () => unknown) => factory(),
  });
  assert.equal(registry.has(source.id), true, 'the just-selected local channel must remain saveable before the membership refresh');
  assert.equal(registry.has(endpoint.id), true);
});

test('production member addition reuses the already loaded route without Save-time lookups', async () => {
  const source = { id: 'source', auth_mode: 'oauth' } as CodexAccount;
  const grant = { id: 'grant', source_account_id: source.id, connected: true, source_binding_valid: true, needs_reauth: false, last_service_mode: 'official' };
  const draft = createCodexServiceMemberDraftHarness({ accounts: [source], grants: [grant], list: async () => { throw new Error('Save must not load grants'); } });
  assert.deepEqual(await draft.add(), [source.id]);
  assert.deepEqual(draft.bindings(), {});
  assert.equal(draft.calls.lists, 0);
  assert.deepEqual(await draft.prepare(), { source: 'grant' });
  assert.deepEqual(draft.calls.choices, [], 'saved official authorization needs no OAuth chooser');
  assert.equal(draft.calls.mutations, 0);
  assert.deepEqual(draft.collection, { accountIds: [], officialAccountBindings: {} }, 'selection changes only the editor draft');
  draft.open(false);
  assert.deepEqual(draft.collection.accountIds, [], 'closing without saving cannot add either channel');
  draft.hook.unmount();
});

test('closing the member dialog during real hook metadata loading fences later choices, mutation and errors', async () => {
  for (const fail of [false, true]) {
    const source = { id: 'source', auth_mode: 'oauth' } as CodexAccount;
    const metadata = deferred<any[]>();
    const draft = createCodexServiceMemberDraftHarness({ accounts: [source], list: () => metadata.promise });
    await draft.add();
    const pending = draft.choose('official'); await settlePromises();
    assert.equal(draft.hook.flush().busy, true);
    draft.open(false);
    if (fail) metadata.reject(new Error('late private metadata failure')); else metadata.resolve([]);
    assert.equal(await pending, undefined);
    assert.equal(draft.calls.mutations, 0);
    assert.deepEqual(draft.calls.choices, []);
    assert.deepEqual(draft.calls.errors, []);
    assert.deepEqual(draft.bindings(), {});
    assert.equal(draft.hook.flush().busy, false);
    assert.deepEqual(draft.collection.accountIds, []);
    draft.hook.unmount();
  }
});

test('production save submits source IDs and only selected official bindings after explicit save', async () => {
  const source = { id: 'source', auth_mode: 'oauth' } as CodexAccount;
  const other = { id: 'other', auth_mode: 'apikey' } as CodexAccount;
  const draft = createCodexServiceMemberDraftHarness({ accounts: [source, other], list: async () => [{
    id: 'grant', source_account_id: source.id, connected: true, source_binding_valid: true, needs_reauth: false, last_service_mode: 'official',
  }] });
  await draft.add(); await draft.choose('official', source.id);
  const h = saveMembersContext({ selected: draft.selected(), accounts: new Map([[source.id, source], [other.id, other]]),
    bindings: { ...draft.bindings(), unselected: 'must-not-submit' }, prepare: () => draft.prepare() });
  assert.equal(h.saved.length, 0, 'a checkbox or authorization never invokes save');
  assert.deepEqual(draft.collection.accountIds, []);
  await h.action();
  assert.equal(h.saved.length, 1);
  assert.deepEqual(Array.from(h.saved[0].accountIds), ['source', 'other']);
  assert.deepEqual(JSON.parse(JSON.stringify(h.saved[0].officialAccountBindings)), { source: 'grant' });
  assert.equal(h.saved[0].accountIds.some((id: string) => id.startsWith('chatgpt_plan_')), false);
  assert.equal(h.closed(), true);
  assert.equal(h.error(), '');
  draft.hook.unmount();
});

test('production save waits for the atomic result and failure keeps source selection and binding draft open', async () => {
  const selected = new Set(['source']);
  const bindings = { source: 'grant' };
  const transaction = deferred<void>();
  const h = saveMembersContext({ selected, accounts: new Map([['source', { id: 'source' }]]), bindings, save: () => transaction.promise });
  const saving = h.action(); await settlePromises();
  assert.equal(h.saved.length, 1);
  assert.equal(h.closed(), false, 'save cannot close before the backend commits');
  transaction.reject(new Error('CHATGPT_PLAN_SERVICE_CHANGED'));
  await saving;
  assert.equal(h.closed(), false, 'a rejected atomic save must remain retryable');
  assert.equal(h.error(), 'chatgptPlan.error.serviceChanged');
  assert.deepEqual([...selected], ['source']);
  assert.deepEqual(bindings, { source: 'grant' });
  assert.deepEqual(JSON.parse(JSON.stringify(h.saved[0].officialAccountBindings)), bindings);
});


test('row OAuth resolves before Save; Save submits once and never opens a route chooser', async () => {
  const source = { id: 'source', auth_mode: 'oauth' } as CodexAccount;
  const second = { id: 'second', auth_mode: 'oauth' } as CodexAccount;
  const draft = createCodexServiceMemberDraftHarness({ accounts: [source, second], list: async () => [] }); await draft.add();
  const changing = draft.choose('official', source.id); await settlePromises();
  const transaction = deferred<void>();
  const h = saveMembersContext({ selected: draft.selected(), accounts: new Map([['source', source], ['second', second]]), prepare: () => draft.prepare(), save: () => transaction.promise });
  await h.action(); assert.equal(h.saved.length, 0, 'pending OAuth cannot save a different route');
  const request = draft.calls.choices[0]; assert.deepEqual(request.ids, ['source']);
  draft.emit({ requestId: request.requestId, sourceAccountId: 'source', status: 'selected', mode: 'official', registrationId: 'grant' }); await changing;
  const saving = h.action(); await settlePromises(); await h.action();
  assert.equal(h.saved.length, 1); assert.equal(draft.calls.choices.length, 1, 'Save never opens a new chooser');
  assert.deepEqual(JSON.parse(JSON.stringify(h.saved[0].officialAccountBindings)), { source: 'grant' });
  assert.equal(h.closed(), false); transaction.resolve(); await saving; assert.equal(h.closed(), true); assert.equal(h.pending(), false); draft.hook.unmount();
});
test('cancelling row OAuth keeps membership selected and Save uses its unchanged local route without prompting', async () => {
  const source = { id: 'source', auth_mode: 'oauth' } as CodexAccount;
  const draft = createCodexServiceMemberDraftHarness({ accounts: [source], list: async () => [] }); await draft.add();
  const changing = draft.choose('official'); await settlePromises(); const cancelled = draft.calls.choices[0];
  draft.emit({ requestId: cancelled.requestId, sourceAccountId: 'source', status: 'cancelled' }); await changing;
  assert.deepEqual([...draft.selected()], ['source']); assert.equal(draft.mode(), 'codex');
  const h = saveMembersContext({ selected: draft.selected(), accounts: new Map([['source', source]]), prepare: () => draft.prepare() });
  await h.action(); assert.equal(h.saved.length, 1); assert.equal(h.closed(), true); assert.equal(h.pending(), false);
  assert.deepEqual(JSON.parse(JSON.stringify(h.saved[0].officialAccountBindings)), {});
  assert.equal(draft.calls.choices.length, 1); draft.hook.unmount();
});

test('an old Save completion cannot close or clear the busy state of a reopened editor', async () => {
  const original = deferred<void>(), reopened = deferred<void>(); let saves = 0;
  const h = saveMembersContext({ selected: new Set(['source']), accounts: new Map([['source', { id: 'source' }]]),
    save: () => ++saves === 1 ? original.promise : reopened.promise });
  const first = h.action(); await settlePromises(); h.reopen();
  const second = h.action(); await settlePromises();
  assert.equal(h.saved.length, 2); assert.equal(h.pending(), true);
  original.resolve(); await first;
  assert.equal(h.closed(), false); assert.equal(h.pending(), true);
  reopened.resolve(); await second;
  assert.equal(h.closed(), true); assert.equal(h.pending(), false);
});


test('member settings save each explicit mapping choice independently of session affinity', async () => {
  for (const enabled of [false, true]) {
    const h = saveMembersContext({ mappingEnabled: enabled, selected: new Set(['source']), accounts: new Map([['source', { id: 'source' }]]) });
    await h.action();
    assert.equal(h.saved.length, 1);
    assert.equal(h.saved[0].sessionIdentityMappingEnabled, enabled);
    assert.equal(h.saved[0].sessionAffinity, true);
    assert.equal(h.closed(), true);
  }
});
