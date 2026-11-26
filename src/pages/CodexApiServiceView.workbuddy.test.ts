import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { deferred, loadHookModule } from '../../tests/helpers/reactHookHarness';

const rotationCode = 'WORKBUDDY_DELEGATE_KEY_ROTATION_UNSUPPORTED';
const messageKey = 'workbuddy.nativeCodex.managedKeyRotation';
const managedMessage = '由工具桥管理，撤销配置后重新创建';
const translate = (key: string, fallback?: unknown) => key === messageKey
  ? managedMessage : typeof fallback === 'string' ? fallback : key;
const key = (managed = false) => ({
  id: 'fixture-key', key: 'synthetic-local-key', label: 'Fixture key', enabled: true,
  accountIds: [], workbuddyDelegate: managed ? { providerAccountId: 'fixture-source' } : null,
});
const state = (managed = false) => ({ collection: { enabled: true, apiKeys: [key(managed)] } });
type Node = { type: any; props: Record<string, any> };
function nodes(value: any): Node[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  return value?.props ? [value, ...nodes(value.props.children)] : [];
}
function text(value: any): string {
  if (Array.isArray(value)) return value.map(text).join('');
  if (value?.props) return text(value.props.children);
  return typeof value === 'string' || typeof value === 'number' ? String(value) : '';
}

// Render the complete production View; only its imported leaf widgets/data hooks are stubbed.
function view(managed: boolean, busy = false) {
  const path = new URL('./CodexApiServiceView.tsx', import.meta.url);
  const file = ts.createSourceFile(path.pathname, readFileSync(path, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const imports: Record<string, any> = {};
  for (const statement of file.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
    const name = statement.moduleSpecifier.text;
    if (name === 'react' || name.endsWith('.css')) continue;
    const exports: Record<string, any> = {};
    const bindings = statement.importClause?.namedBindings;
    if (bindings && ts.isNamedImports(bindings)) {
      for (const item of bindings.elements) exports[item.propertyName?.text ?? item.name.text] = item.name.text;
    }
    imports[name] = exports;
  }
  Object.assign(imports['../hooks/useChatGptPlanOverview'], {
    useChatGptPlanOverview: () => ({ accounts: [], displaySourceIds: new Set() }),
  });
  Object.assign(imports['./CodexOverloadHistory'], { useCodexOverloadCounts: () => ({}) });
  Object.assign(imports['../utils/chatgptPlanOverview'], { codexServiceDisplayRows: () => [] });
  Object.assign(imports['../utils/codexServiceAccountStats'], {
    aggregateCodexServiceAccountStats: () => new Map(), codexServiceAccountSources: () => new Map(),
  });
  Object.assign(imports['../components/codex/codexLocalAccessModalModel'], { summarizeAccountPoolHealth: () => ({}) });
  Object.assign(imports['../utils/codexLocalAccessStatus'], {
    resolveCodexLocalAccessRuntimeStatus: () => 'running', CODEX_LOCAL_ACCESS_STATUS_KEYS: { running: 'running' },
  });
  Object.assign(imports['../utils/codexGatewayKeyRecovery'], { isCodexGatewayKeyRecoveryError: () => false });
  Object.assign(imports['../utils/platformMeta'], { getPlatformLabel: () => 'Codex' });
  Object.assign(imports['../utils/codexApiKeyAccountScope'], { selectCodexApiKeyScopeAccounts: () => [] });
  Object.assign(imports['../utils/codexServiceMemberView'], { countCodexServiceSourceAccounts: () => 0 });
  Object.assign(imports['../hooks/useCodexApiKeyInspection'], { codexApiKeyInspectionCardId: (id: string) => `key-${id}` });
  const h = loadHookModule(path, imports);
  const rotations: string[] = [];
  const props: any = {
    t: translate, activeTab: 'keys', busy, serviceTabs: [], memberAccounts: [], accounts: [],
    localAccessAccounts: [], memberIds: [], statsTimeRange: { startAt: 0, endAt: 1 },
    state: { running: true }, collection: { ...state(managed).collection, accountIds: [] },
    apiKeyDrafts: {}, apiKeyPolicyDrafts: {}, apiKeyStatsById: new Map(), expandedApiKeyPolicyIds: new Set(),
    apiKeyPolicyDraftFromValue: () => ({ inheritAccountPool: false, accountIds: [] }),
    apiKeyInheritsAccountPool: () => false, apiKeyHasFixedAccountScope: () => false,
    apiKeyPolicyDraftIsDirty: () => false, formatDateTime: () => '--',
    formatCompactNumber: String, formatAccountTokenUsage: () => '0', formatUsdCost: () => '$0',
    handleRotateApiKey: (id: string) => rotations.push(id),
  };
  const tree = nodes(h.render(() => h.exports.CodexApiServiceView(props)));
  const card = tree.find((node) => node.props.id === 'key-fixture-key');
  assert.ok(card);
  const button = nodes(card).find((node) => node.type === 'button' &&
    (node.props.title === managedMessage || node.props.title === 'codex.localAccess.rotateKey'));
  assert.ok(button);
  return { h, tree, button, rotations };
}

// Execute production hook declarations and both production action handlers, like Page.refresh.test.ts.
function controller(managed = false) {
  const path = new URL('./CodexApiServicePage.tsx', import.meta.url);
  const file = ts.createSourceFile(path.pathname, readFileSync(path, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const owner = file.statements.find((node): node is ts.FunctionDeclaration =>
    ts.isFunctionDeclaration(node) && node.name?.text === 'useCodexApiServicePageController');
  assert.ok(owner?.body);
  const wanted = new Set(['state', 'latestStateRef', 'busy', 'error', 'notice', 'runAction', 'handleRotateApiKey']);
  function bindings(node: ts.BindingName): string[] {
    return ts.isIdentifier(node) ? [node.text] : node.elements.flatMap((element) => ts.isBindingElement(element) ? bindings(element.name) : []);
  }
  const statements = owner.body.statements.filter((node) => ts.isVariableStatement(node)
    ? node.declarationList.declarations.some((decl) => bindings(decl.name).some((name) => wanted.has(name)))
    : ts.isExpressionStatement(node) && node.getText(file) === 'latestStateRef.current = state;');
  const h = loadHookModule(new URL('../hooks/useRefreshSafeDraft.ts', import.meta.url), {});
  const calls: string[] = [];
  const confirmations: unknown[] = [];
  let confirmation: Promise<boolean> = Promise.resolve(true);
  let failure: unknown;
  const context = {
    ...h.hooks, t: translate, presentWindowsOperationError: () => false,
    confirmDialog: (...args: unknown[]) => { confirmations.push(args); return confirmation; },
    codexLocalAccessService: { async rotateCodexLocalAccessNamedApiKey(id: string) {
      calls.push(id); if (failure) throw failure;
      return state();
    } },
  };
  const source = `(() => { ${statements.map((node) => node.getText(file)).join('\n')}
    return { state, setState, error, notice, busy, handleRotateApiKey }; })`;
  const render = vm.runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  h.render(render);
  h.flush().setState(state(managed)); h.flush();
  return { h, calls, confirmations, read: () => h.flush(),
    change(managed: boolean) { h.flush().setState(state(managed)); h.flush(); },
    confirmWith(value: Promise<boolean>) { confirmation = value; },
    failWith(error: unknown) { failure = error; },
  };
}

test('production key View disables managed rotation and exposes a visible accessible localized explanation', () => {
  const p = view(true);
  assert.equal(p.button.props.disabled, true);
  assert.equal(p.button.props['aria-label'], managedMessage);
  assert.equal(p.button.props.title, managedMessage);
  const explanation = p.tree.find((node) => node.props.id === p.button.props['aria-describedby']);
  assert.ok(explanation);
  assert.equal(text(explanation), managedMessage);
  assert.deepEqual(p.rotations, []);
  p.h.unmount();
});

test('production key View keeps ordinary rotation available and preserves busy disabling', () => {
  const p = view(false);
  assert.equal(p.button.props.disabled, false);
  assert.equal(p.button.props['aria-describedby'], undefined);
  p.button.props.onClick();
  assert.deepEqual(p.rotations, ['fixture-key']);
  p.h.unmount();
  const busy = view(false, true);
  assert.equal(busy.button.props.disabled, true);
  busy.h.unmount();
});

test('production managed handler refuses before confirmation and IPC and reports localized guidance', async () => {
  const p = controller(true);
  await p.read().handleRotateApiKey('fixture-key');
  assert.deepEqual(p.confirmations, []); assert.deepEqual(p.calls, []);
  assert.equal(p.read().error, managedMessage); assert.equal(p.read().busy, false);
  assert.equal(p.read().notice, '');
  p.h.unmount();
});

test('production ordinary handler preserves confirmation and successful IPC', async () => {
  const p = controller();
  await p.read().handleRotateApiKey('fixture-key');
  assert.equal(p.confirmations.length, 1); assert.deepEqual(p.calls, ['fixture-key']);
  assert.equal(p.read().error, ''); assert.equal(p.read().notice, 'API Key 已重置');
  p.h.unmount();
});

test('an older rendered handler consults current managed state rather than its original closure', async () => {
  const p = controller();
  const old = p.read().handleRotateApiKey;
  p.change(true);
  await old('fixture-key');
  assert.deepEqual(p.confirmations, []); assert.deepEqual(p.calls, []);
  assert.equal(p.read().error, managedMessage);
  p.h.unmount();
});

test('state changed while confirmation is pending is rechecked before IPC', async () => {
  const p = controller();
  const pending = deferred<boolean>(); p.confirmWith(pending.promise);
  const rotation = p.read().handleRotateApiKey('fixture-key');
  assert.equal(p.confirmations.length, 1);
  p.change(true); pending.resolve(true); await rotation;
  assert.deepEqual(p.calls, []); assert.equal(p.read().error, managedMessage);
  p.h.unmount();
});

test('exact backend rotation errors map to localized guidance without a success notice', async () => {
  for (const failure of [rotationCode, new Error(rotationCode)]) {
    const p = controller(); p.failWith(failure);
    await p.read().handleRotateApiKey('fixture-key');
    assert.deepEqual(p.calls, ['fixture-key']); assert.equal(p.read().error, managedMessage);
    assert.equal(p.read().notice, ''); assert.equal(p.read().busy, false);
    p.h.unmount();
  }
});

test('unrelated backend errors containing the code are not reclassified', async () => {
  const p = controller(); const failure = `${rotationCode}: unrelated detail`; p.failWith(failure);
  await p.read().handleRotateApiKey('fixture-key');
  assert.equal(p.read().error, failure); assert.equal(p.read().notice, '');
  p.h.unmount();
});
