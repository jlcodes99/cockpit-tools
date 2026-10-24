import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

// Exercise the production handlers without mounting their unrelated page UI.
function handler(file: string, name: string, context: Record<string, unknown>) {
  const source = ts.createSourceFile(file, readFileSync(new URL(file, import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let initializer: ts.Expression | undefined;
  function visit(node: ts.Node) {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === name) {
      initializer = node.initializer;
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(initializer, `Missing production handler: ${name}`);
  const callback = ts.isCallExpression(initializer) ? initializer.arguments[0] : initializer;
  const compiled = ts.transpileModule(`(${callback.getText(source)})`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return vm.runInNewContext(compiled, context) as (...args: unknown[]) => Promise<unknown>;
}

function harness(file: string, name: string, enabled: boolean, confirmResult = true) {
  const calls: { enabled: boolean[]; activate: unknown[]; confirm: string[]; busy: boolean[] } = {
    enabled: [], activate: [], confirm: [], busy: [],
  };
  const next = { collection: { enabled: !enabled } };
  const state: { value: unknown } = { value: null };
  const context = {
    collection: { enabled }, localAccessCollection: { enabled },
    mountedRef: { current: true },
    t: (key: string) => key,
    console: { info() {}, error() {} }, performance: { now: () => 0 },
    confirmDialog: async (message: string) => { calls.confirm.push(message); return confirmResult; },
    codexLocalAccessService: {
      setCodexLocalAccessEnabled: async (value: boolean) => { calls.enabled.push(value); return next; },
      activateCodexLocalAccess: async (instanceId?: unknown) => { calls.activate.push(instanceId); return next; },
    },
    setState: (value: unknown) => { state.value = value; },
    setLocalAccessState: (value: unknown) => { state.value = value; },
    setLocalAccessSaving: (value: boolean) => calls.busy.push(value),
    setLocalAccessStarting: (value: boolean) => calls.busy.push(value),
    setActivating: (value: boolean) => calls.busy.push(value),
    setLocalAccessLaunchCurrent() {}, setMessage() {}, setError() {}, setNotice() {},
    fetchCurrentAccount: async () => {}, refreshApiServiceCurrent: async () => {},
    runAction: async (action: () => Promise<void>) => { await action(); return true; },
    presentWindowsOperationError: () => false,
  };
  return { action: handler(file, name, context), calls, state, next };
}

for (const [file, toggleName, activateName] of [
  ['./useCodexAccountsLocalAccessController.tsx', 'handleToggleLocalAccessEnabled', 'handleActivateLocalAccess'],
  ['./CodexApiServicePage.tsx', 'handleToggleEnabled', 'handleActivateService'],
]) {
  test(`${file}: enabling and disabling call the service without the retired risk prompt`, async () => {
    for (const enabled of [false, true]) {
      const h = harness(file, toggleName, enabled, false);
      await h.action();
      assert.deepEqual(h.calls.enabled, [!enabled]);
      assert.deepEqual(h.calls.confirm, []);
      assert.equal(h.state.value, h.next);
    }
  });

  test(`${file}: activating an enabled service proceeds without risk acknowledgement`, async () => {
    const h = harness(file, activateName, true, false);
    await h.action({ instanceId: 'instance-a' });
    assert.equal(h.calls.activate.length, 1);
    assert.deepEqual(h.calls.confirm, []);
    assert.equal(h.state.value, h.next);
    assert.deepEqual(h.calls.busy, [true, false]);
  });

  test(`${file}: declining enable-and-switch still prevents activation`, async () => {
    const h = harness(file, activateName, false, false);
    await h.action();
    assert.deepEqual(h.calls.confirm, ['codex.localAccess.enableBeforeActivateMessage']);
    assert.deepEqual(h.calls.activate, []);
    assert.deepEqual(h.calls.busy, []);
  });

  test(`${file}: accepting enable-and-switch activates without a second prompt`, async () => {
    const h = harness(file, activateName, false);
    await h.action();
    assert.deepEqual(h.calls.confirm, ['codex.localAccess.enableBeforeActivateMessage']);
    assert.equal(h.calls.activate.length, 1);
    assert.equal(h.state.value, h.next);
    assert.deepEqual(h.calls.busy, [true, false]);
  });
}

test('custom model saves preserve per-account upstream names and existing global rules', async () => {
  const modelAliases = [{ name: 'existing', alias: 'public', fork: true }];
  const excludedModels = ['excluded-model'];
  const models = [
    { clientModel: 'shared-model', accountId: 'account-a', upstreamModel: 'upstream-a' },
    { clientModel: 'shared-model', accountId: 'account-b', upstreamModel: 'upstream-b' },
  ];
  const next = { collection: { modelAliases, excludedModels, customModels: models } };
  const writes: unknown[][] = [];
  let savedState: unknown;
  const save = handler('./CodexApiServicePage.tsx', 'handleSaveCustomModels', {
    collection: { modelAliases, excludedModels },
    codexLocalAccessService: {
      updateCodexLocalAccessModelRules: async (...args: unknown[]) => { writes.push(args); return next; },
    },
    runAction: async (action: () => Promise<void>) => action(),
    setState: (value: unknown) => { savedState = value; },
    t: (key: string) => key,
  });

  await save(models);
  assert.deepEqual(writes[0], [modelAliases, excludedModels, models]);
  assert.equal(savedState, next);

  await save([]);
  assert.deepEqual(writes[1], [modelAliases, excludedModels, []]);
});

test('global model rule saves leave custom bindings unchanged', async () => {
  const writes: unknown[][] = [];
  const modelAliases = [{ name: 'existing', alias: 'public', fork: true }];
  const excludedModels = ['excluded-model'];
  const save = handler('./CodexApiServicePage.tsx', 'handleSaveModelRules', {
    modelAliasesText: 'existing=public', excludedModelsText: 'excluded-model',
    parseModelAliasText: () => modelAliases,
    parseModelRuleText: () => excludedModels,
    codexLocalAccessService: {
      updateCodexLocalAccessModelRules: async (...args: unknown[]) => { writes.push(args); return {}; },
    },
    runAction: async (action: () => Promise<void>) => action(),
    setState() {}, t: (key: string) => key,
  });

  await save();
  assert.deepEqual(writes, [[modelAliases, excludedModels]]);
});
