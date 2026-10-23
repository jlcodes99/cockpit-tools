import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const require = createRequire(import.meta.url);
function parse(file: string) {
  return ts.createSourceFile(file, readFileSync(new URL(file, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
}
function all(root: ts.Node, match: (node: ts.Node) => boolean): ts.Node[] {
  const result: ts.Node[] = [];
  const visit = (node: ts.Node) => { if (match(node)) result.push(node); ts.forEachChild(node, visit); };
  visit(root); return result;
}
function evaluate(code: string, context: Record<string, unknown>) {
  return vm.runInNewContext(ts.transpileModule(code, { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS } }).outputText, { require, exports: {}, ...context });
}
for (const file of ['./useCodexAccountsBaseController.tsx', './useCodexAccountsAccessController.tsx']) {
  test(`${file}: explicit import adds unbound accounts locally without requesting authorization`, async () => {
    const source = parse(file);
    const condition = file.includes('Base') ? 'explicitApiServiceChoiceRef.current' : 'options.addToApiService';
    const block = all(source, (node) => ts.isIfStatement(node) && node.expression.getText(source) === condition)[0] as ts.IfStatement;
    assert.ok(block);
    const imported = [{ id: 'a', oauth: true }, { id: 'b', oauth: true }, { id: 'api', oauth: false }];
    const requested: string[][] = [];
    const appended: string[][] = [];
    const run = evaluate(`(async () => { let accountIds = ['a', 'b', 'api']; ${block.thenStatement.getText(source)} return accountIds; })`, {
      explicitApiServiceChoiceRef: { current: true }, result: { imported },
      failures: [],
      codexService: { listCodexAccounts: async () => imported },
      listChatGptPlanAccounts: async () => [], canChooseCodexServiceAuthorization: (account: { oauth: boolean }) => account.oauth,
      localAccessState: { collection: { accountIds: [] } }, localAccessCollection: { accountIds: [] },
      prepareCodexServiceJoins: async (accounts: typeof imported) => ({
        localIds: accounts.map((account) => account.id), resumedIds: [], failures: [],
      }),
      t: (key: string) => key,
      requestCodexServiceAuthorizations: (ids: string[]) => requested.push(Array.from(ids)),
      codexLocalAccessService: { appendCodexLocalAccessAccounts: async (ids: string[]) => { appended.push(Array.from(ids)); return { state: {}, syncedAccountIds: ids }; } },
      setLocalAccessSaving() {}, setLocalAccessState() {}, setImportApiServiceGuideCount() {},
      ensureLocalAccessEntryVisible: async () => {}, window: { dispatchEvent() {} }, Event,
    });
    const remaining = await run();
    assert.deepEqual(requested, []);
    if (file.includes('Base')) assert.deepEqual(Array.from(remaining), ['a', 'b', 'api']);
    else assert.deepEqual(appended, [['a', 'b', 'api']]);
  });
}

for (const file of ['./useCodexAccountsBaseController.tsx', './useCodexAccountsAccessController.tsx']) {
  for (const localIds of [['before', 'after'], []]) {
    test(`${file}: report stale authorization after committing ${localIds.length ? 'other local accounts' : 'other official accounts'}`, async () => {
      const source = parse(file);
      const base = file.includes('Base');
      const imported = ['before', 'stale', 'after'].map((id) => ({ id, email: `${id}@example.test` }));
      const prepared = {
        localIds, resumedIds: ['official'],
        failures: [{ accountId: 'stale', error: new Error('CHATGPT_PLAN_REAUTH_REQUIRED') }],
      };
      const appended: string[][] = [], states: unknown[] = [], events: string[] = [];
      const state = { collection: { accountIds: ['official', ...localIds] } };
      const context = {
        useCallback: (action: unknown) => action,
        explicitApiServiceChoiceRef: { current: true }, syncImportedToApiService: true,
        result: { imported }, codexService: { listCodexAccounts: async () => imported },
        localAccessState: { collection: { accountIds: [] } }, localAccessCollection: { accountIds: [] },
        listChatGptPlanAccounts: async () => [], canChooseCodexServiceAuthorization: () => true,
        prepareCodexServiceJoins: async () => prepared,
        implicitCodexServiceAccountIds: (ids: string[]) => ids,
        maskAccountText: (text: string) => text.replace('@example.test', '@masked'),
        resolveCodexAccountDisplayName: (account: { email: string }) => account.email,
        chatGptPlanErrorKey: () => 'chatgptPlan.error.reauthRequired',
        t: (key: string, fallback?: string) => key === 'chatgptPlan.error.reauthRequired' ? 'Please reauthorize' : fallback ?? key,
        codexLocalAccessService: { appendCodexLocalAccessAccounts: async (ids: string[]) => {
          appended.push(Array.from(ids)); events.push('append'); return { state, syncedAccountIds: ids };
        } },
        setLocalAccessState: (value: unknown) => { states.push(value); events.push('state'); },
        ensureLocalAccessEntryVisible: async () => {}, setImportApiServiceGuideCount() {}, setLocalAccessSaving() {},
        window: { dispatchEvent() {} }, Event,
      };
      let message: string;
      if (base) {
        const declaration = all(source, (node) => ts.isVariableDeclaration(node) && node.name.getText(source) === 'syncImportedAccountsToApiService')[0] as ts.VariableDeclaration;
        const run = evaluate(`(${declaration.initializer!.getText(source)})`, context);
        try { await run(imported.map((account) => account.id)); assert.fail('partial failure must reach the import error handler'); }
        catch (error) { message = String(error); }
      } else {
        const block = all(source, (node) => ts.isIfStatement(node) && node.expression.getText(source) === 'options.addToApiService')[0] as ts.IfStatement;
        const run = evaluate(`(async () => { let apiServiceError = null; ${block.thenStatement.getText(source)} return apiServiceError; })`, context);
        message = await run();
      }
      assert.match(message!, /stale@masked: Please reauthorize/);
      assert.doesNotMatch(message!, /CHATGPT_PLAN_REAUTH_REQUIRED/);
      assert.deepEqual(appended, localIds.length ? [localIds] : []);
      assert.deepEqual(states, localIds.length ? [state] : []);
      assert.deepEqual(events, localIds.length ? ['append', 'state'] : []);
    });
  }
}

test('compact and table add actions carry the same official badge and use the shared join handler', () => {
  const source = parse('./useCodexAccountsLocalAccessController.tsx');
  const declaration = all(source, (node) => ts.isVariableDeclaration(node) && node.name.getText(source) === 'renderAddLocalAccessAccountButton')[0] as ts.VariableDeclaration;
  assert.ok(declaration.initializer);
  const requests: string[] = [];
  const grant = { id: 'grant' };
  const ids = new Set<string>();
  const jsxRuntime = require('react/jsx-runtime');
  const render = evaluate(`(${declaration.initializer.getText(source)})`, {
    canDirectlyAddLocalAccessAccount: () => true, t: (key: string) => key,
    addingLocalAccessAccountId: null, handleAddLocalAccessAccount: (id: string) => requests.push(id),
    planOverview: { accounts: [grant] }, localAccessAccountIdSet: ids,
    CodexAccountOfficialBadge: 'OfficialBadge', RefreshCw: 'Spinner', Link2: 'Link',
    jsx_runtime_1: jsxRuntime,
  });
  const account = { id: 'source' };
  const node = render(account, 'action-btn');
  assert.match(node.props.className, /codex-official-add-action/);
  const badge = node.props.children.find((child: any) => child?.type === 'OfficialBadge');
  assert.equal(badge.props.account, account);
  assert.equal(badge.props.memberIds, ids);
  assert.equal(badge.props.grants[0], grant);
  node.props.onClick();
  assert.deepEqual(requests, ['source']);
});

test('production quick add reuses an existing official session without showing authorization or appending the local account', async () => {
  const file = parse('./useCodexAccountsLocalAccessController.tsx');
  const declaration = all(file, (node) => ts.isVariableDeclaration(node) && node.name.getText(file) === 'handleAddLocalAccessAccount')[0] as ts.VariableDeclaration;
  const source = { id: 'source', auth_mode: 'oauth' };
  const state = { collection: { accountIds: ['chatgpt_plan_grant'] } };
  const requested: string[] = [], appended: string[][] = [], states: unknown[] = [];
  const render = evaluate(`(${declaration.initializer!.getText(file)})`, {
    useCallback: (action: unknown) => action, accounts: [source], addingLocalAccessAccountId: null,
    localAccessAccountIdSet: new Set(), setAddingLocalAccessAccountId() {},
    canChooseCodexServiceAuthorization: () => true,
    listChatGptPlanAccounts: async () => [{ id: 'grant', connected: true }],
    reuseCodexServiceJoin: async (account: typeof source, grants: { id: string }[]) => {
      assert.equal(account, source); assert.equal(grants[0].id, 'grant');
      return { kind: 'joined', state, selectedIds: state.collection.accountIds };
    },
    requestCodexServiceAuthorization: (id: string) => requested.push(id),
    codexLocalAccessService: { appendCodexLocalAccessAccounts: async (ids: string[]) => { appended.push(ids); } },
    setLocalAccessState: (value: unknown) => states.push(value), ensureLocalAccessEntryVisible: async () => {},
    setMessage() {}, t: (key: string) => key, window: { dispatchEvent() {} }, Event,
  });
  await render(source.id);
  assert.deepEqual(requested, []);
  assert.deepEqual(appended, []);
  assert.deepEqual(states, [state]);
});
