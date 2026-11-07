import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

// Run the production initialization effect without operating a desktop or browser.
function createEditor() {
  const source = ts.createSourceFile('modal.tsx', readFileSync(new URL('./CodexLocalAccessModal.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let effect: ts.CallExpression | undefined;
  function visit(node: ts.Node) {
    if (ts.isCallExpression(node) && node.expression.getText(source) === 'useEffect'
      && node.arguments[0]?.getText(source).includes('setCustomRoutingOpen(false)')) effect = node;
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(effect);
  const state: Record<string, any> = {};
  const context: Record<string, any> = {
    isOpen: true, mode: 'members', membersDraftDirty: false, customRoutingOpen: false,
    modalSessionRef: { current: null },
    collection: { accountIds: ['account'], customRoutingRules: [{ accountId: 'account', priority: 2, weight: 3 }] },
    normalizedInitialSelectedIds: ['account'], initialRestrictFreeAccounts: true,
    initialSessionIdentityMappingEnabled: true, initialSessionAffinity: true, initialSessionAffinityTtlSeconds: 3600,
    CUSTOM_ROUTING_PRIORITY_MIN: 0, CUSTOM_ROUTING_WEIGHT_MIN: 1,
    normalizeCustomRoutingPriority: (value: number) => value, normalizeCustomRoutingWeight: (value: number) => value,
    window: { setTimeout() {} }, searchInputRef: { current: null },
  };
  const callbacks = new Set(effect.arguments[0].getText(source).match(/\bset[A-Z]\w+/g));
  for (const name of callbacks) context[name] = (value: any) => {
    const key = name.slice(3);
    state[key] = typeof value === 'function' ? value(state[key]) : value;
    if (name === 'setCustomRoutingOpen') context.customRoutingOpen = state[key];
  };
  const compile = (expression: string) => vm.runInNewContext(ts.transpileModule(expression, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText, context);
  const initialize = compile(`(${effect.arguments[0].getText(source)})`);
  let previous: unknown[] | undefined;
  function render() {
    const deps: unknown[] = compile(`(${effect!.arguments[1].getText(source)})`);
    if (previous && deps.every((value, index) => Object.is(value, previous![index]))) return;
    previous = deps;
    initialize();
  }
  render();
  return { state, context, render, edit() {
    context.customRoutingOpen = state.CustomRoutingOpen = true;
    state.CustomRoutingDraft.account = { priority: 17, weight: 6, isBackup: true, isPreferred: false };
    state.CustomRoutingSelected = new Set(['account']);
    state.CustomRoutingQuery = 'search'; state.CustomRoutingBulkPriority = '25';
    state.CustomRoutingError = 'save failed';
    state.PortInput = '9000'; state.UpstreamProxyDraftUrl = 'http://edited.invalid';
    render();
  } };
}

test('background collection and quota refreshes preserve the custom routing dialog and unsaved edits', () => {
  const h = createEditor(); h.edit();
  for (let i = 0; i < 3; i++) {
    h.context.collection = structuredClone(h.context.collection);
    h.context.normalizedInitialSelectedIds = ['account'];
    h.render();
    assert.equal(h.state.CustomRoutingOpen, true);
    assert.equal(h.state.CustomRoutingDraft.account.priority, 17);
    assert.equal(h.state.CustomRoutingDraft.account.weight, 6);
    assert.equal(h.state.CustomRoutingDraft.account.isBackup, true);
    assert.deepEqual([...h.state.CustomRoutingSelected], ['account']);
    assert.equal(h.state.CustomRoutingQuery, 'search');
    assert.equal(h.state.CustomRoutingBulkPriority, '25');
    assert.equal(h.state.CustomRoutingError, 'save failed');
    assert.equal(h.state.PortInput, '9000');
    assert.equal(h.state.UpstreamProxyDraftUrl, 'http://edited.invalid');
  }
});

test('member draft changes cannot close an open custom routing editor', () => {
  const h = createEditor(); h.edit();
  h.context.membersDraftDirty = true; h.render();
  assert.equal(h.state.CustomRoutingOpen, true);
  assert.equal(h.state.CustomRoutingDraft.account.priority, 17);
});

test('reopening or switching modes initializes a fresh session with the latest saved rules', () => {
  const h = createEditor(); h.edit();
  h.context.membersDraftDirty = true;
  h.context.isOpen = false; h.render();
  h.context.collection.customRoutingRules[0].priority = 8;
  h.context.isOpen = true; h.render();
  assert.equal(h.state.CustomRoutingOpen, false);
  assert.equal(h.state.CustomRoutingDraft.account.priority, 8);
  assert.equal(h.state.CustomRoutingQuery, '');
  h.edit(); h.context.mode = 'panel'; h.render();
  assert.equal(h.state.CustomRoutingOpen, false);
  assert.equal(h.state.CustomRoutingDraft.account.priority, 8);
});

test('late account data still hydrates an untouched member draft', () => {
  const h = createEditor();
  h.context.normalizedInitialSelectedIds = ['account', 'late-account'];
  h.context.collection = { accountIds: ['account', 'late-account'], customRoutingRules: [] };
  h.render();
  assert.deepEqual([...h.state.Selected], ['account', 'late-account']);
  assert.equal(h.state.CustomRoutingDraft['late-account'].weight, 1);
});
