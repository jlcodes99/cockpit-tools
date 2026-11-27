import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { deferred, loadHookModule } from '../../tests/helpers/reactHookHarness';

const code = 'WORKBUDDY_DELEGATE_KEY_ROTATION_UNSUPPORTED';
const message = '由工具桥管理，撤销配置后重新创建';
const key = (managed: boolean, enabled = true, id = managed ? 'managed' : 'ordinary') => ({
  id, key: `synthetic-${id}`, enabled, workbuddyDelegate: managed ? { providerAccountId: 'fixture-source' } : null,
});
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

// Execute the actual Modal selection/guard/action declarations and its production key-card JSX.
// Unrelated membership/statistics effects are excluded, like Modal.refresh.test.ts.
function modal(keys: ReturnType<typeof key>[]) {
  const path = new URL('./CodexLocalAccessModal.tsx', import.meta.url);
  const file = ts.createSourceFile(path.pathname, readFileSync(path, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const owner = file.statements.find((node): node is ts.FunctionDeclaration =>
    ts.isFunctionDeclaration(node) && node.name?.text === 'CodexLocalAccessModal');
  const selector = file.statements.find((node): node is ts.FunctionDeclaration =>
    ts.isFunctionDeclaration(node) && node.name?.text === 'isManagedWorkbuddyRotationTarget');
  assert.ok(owner?.body); assert.ok(selector);
  const wanted = new Set(['collection', 'latestRotationCollectionRef', 'managedRotationNotice',
    'error', 'notice', 'keyVisible', 'copiedField', 'runAction', 'handleResetKey']);
  function bindings(node: ts.BindingName): string[] {
    return ts.isIdentifier(node) ? [node.text] : node.elements.flatMap((element) => ts.isBindingElement(element) ? bindings(element.name) : []);
  }
  const statements = owner.body.statements.filter((node) => ts.isVariableStatement(node)
    ? node.declarationList.declarations.some((decl) => bindings(decl.name).some((name) => wanted.has(name)))
    : ts.isExpressionStatement(node) && node.getText(file) === 'latestRotationCollectionRef.current = collection;');
  let card: ts.JsxElement | undefined;
  function visit(node: ts.Node) {
    if (ts.isJsxElement(node) && node.openingElement.attributes.properties.some((attribute) =>
      ts.isJsxAttribute(attribute) && attribute.name.getText(file) === 'className' &&
      attribute.initializer && ts.isStringLiteral(attribute.initializer) &&
      attribute.initializer.text === 'codex-local-access-config-card codex-local-access-config-card-key')) card = node;
    ts.forEachChild(node, visit);
  }
  visit(owner); assert.ok(card);
  const h = loadHookModule(new URL('../hooks/useRefreshSafeDraft.ts', import.meta.url), {});
  const confirmations: unknown[] = [], calls: string[] = [];
  let confirmation = Promise.resolve(true), failure: unknown;
  let current = { collection: { apiKeys: keys, apiKey: 'synthetic-displayed-key' } };
  const globals = {
    ...h.hooks, exports: {}, require: createRequire(import.meta.url), saving: false, testing: false, starting: false,
    t: (name: string, fallback?: string) => name === 'workbuddy.nativeCodex.managedKeyRotation' ? message : fallback ?? name,
    Eye: 'eye-icon', EyeOff: 'eye-off-icon', Check: 'check-icon', Copy: 'copy-icon', RefreshCw: 'refresh-icon',
    confirmDialog(...args: unknown[]) { confirmations.push(args); return confirmation; },
    async onRotateApiKey() { calls.push('rotate'); if (failure) throw failure; },
  };
  const source = `${selector.getText(file)}\n((state) => { ${statements.map((node) => node.getText(file)).join('\n')}
    return { error, notice, keyVisible, handleResetKey, card: (${card.getText(file)}) }; })`;
  const compiled = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX,
  } }).outputText;
  const render = vm.runInNewContext(compiled, globals);
  h.render(() => render(current));
  return { h, calls, confirmations, read: () => h.flush(),
    button() {
      const button = nodes(h.flush().card).find((node) => node.type === 'button' && text(node) === '重置密钥');
      assert.ok(button); return button;
    },
    change(next: ReturnType<typeof key>[]) { current = { collection: { ...current.collection, apiKeys: next } }; h.flush(); },
    confirmWith(next: Promise<boolean>) { confirmation = next; },
    failWith(next: unknown) { failure = next; },
  };
}

test('production Modal disables a first-enabled managed target and exposes accessible visible guidance', async () => {
  const p = modal([key(false, false), key(true)]);
  const button = p.button();
  assert.equal(button.props.disabled, true); assert.equal(button.props.title, message);
  const note = nodes(p.read().card).find((node) => node.props.id === button.props['aria-describedby']);
  assert.ok(note); assert.equal(text(note), message);
  await p.read().handleResetKey();
  assert.deepEqual(p.calls, []); assert.deepEqual(p.confirmations, []);
  assert.equal(p.read().error, message); assert.equal(p.read().keyVisible, false);
  p.h.unmount();
});

test('production Modal follows the backend all-disabled first-item fallback', async () => {
  const p = modal([key(true, false), key(false, false)]);
  assert.equal(p.button().props.disabled, true);
  await p.read().handleResetKey();
  assert.deepEqual(p.calls, []); assert.deepEqual(p.confirmations, []);
  p.h.unmount();
});

test('a later managed key does not block the ordinary first-enabled target', async () => {
  const p = modal([key(true, false), key(false), key(true, true, 'later-managed')]);
  assert.equal(p.button().props.disabled, false);
  await p.read().handleResetKey();
  assert.deepEqual(p.calls, ['rotate']); assert.equal(p.confirmations.length, 1);
  assert.equal(p.read().notice, 'API 服务密钥已重置'); assert.equal(p.read().keyVisible, true);
  p.h.unmount();
});

test('legacy no-named-key collections retain ordinary reset behavior', async () => {
  const p = modal([]);
  assert.equal(p.button().props.disabled, false);
  await p.read().handleResetKey();
  assert.deepEqual(p.calls, ['rotate']);
  p.h.unmount();
});

test('a stale Modal handler consults current selection before opening confirmation', async () => {
  const p = modal([key(false)]); const old = p.read().handleResetKey;
  p.change([key(true)]);
  await old();
  assert.deepEqual(p.calls, []); assert.deepEqual(p.confirmations, []);
  assert.equal(p.read().error, message);
  p.h.unmount();
});

test('Modal rechecks target changes while confirmation is pending', async () => {
  const p = modal([key(false)]); const pending = deferred<boolean>(); p.confirmWith(pending.promise);
  const rotation = p.read().handleResetKey();
  assert.equal(p.confirmations.length, 1);
  p.change([key(false, false), key(true)]); pending.resolve(true); await rotation;
  assert.deepEqual(p.calls, []); assert.equal(p.read().error, message);
  assert.equal(p.read().notice, ''); assert.equal(p.read().keyVisible, false);
  p.h.unmount();
});

test('Modal maps exact backend code strings and Error objects for both parent callers', async () => {
  for (const failure of [code, new Error(code)]) {
    const p = modal([key(false)]); p.failWith(failure);
    await p.read().handleResetKey();
    assert.deepEqual(p.calls, ['rotate']); assert.equal(p.read().error, message);
    assert.equal(p.read().notice, ''); assert.equal(p.read().keyVisible, false);
    p.h.unmount();
  }
});

test('Modal preserves unrelated backend errors rather than substring-matching the code', async () => {
  const p = modal([key(false)]); const failure = `${code}: unrelated detail`; p.failWith(failure);
  await p.read().handleResetKey();
  assert.equal(p.read().error, failure);
  p.h.unmount();
});
