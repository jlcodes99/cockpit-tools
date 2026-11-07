import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { deferred } from '../../tests/helpers/reactHookHarness.ts';
import { resolveRoutingCatalog } from '../utils/codexModelRoutingValue.ts';

const source = ts.createSourceFile('InstancesManager.tsx', readFileSync(
  new URL('./InstancesManager.tsx', import.meta.url), 'utf8',
), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let enableHandler: ts.VariableDeclaration | undefined;
let editorDisabled: ts.Expression | undefined;
const visit = (node: ts.Node) => {
  if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'handleFormModelCatalogEnabledChange') {
    enableHandler = node;
  }
  if (ts.isJsxSelfClosingElement(node) && node.tagName.getText(source) === 'CodexExperimentalModelEditor') {
    const disabled = node.attributes.properties.find((attribute): attribute is ts.JsxAttribute =>
      ts.isJsxAttribute(attribute) && attribute.name.getText(source) === 'disabled');
    assert.ok(disabled?.initializer && ts.isJsxExpression(disabled.initializer));
    editorDisabled = disabled.initializer.expression;
  }
  ts.forEachChild(node, visit);
};
visit(source);
assert.ok(enableHandler?.initializer && editorDisabled);
const compiled = ts.transpileModule(`globalThis.enable = ${enableHandler.initializer.getText(source)};
globalThis.editorDisabled = () => ${editorDisabled.getText(source)};`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function harness(confirmed: () => Promise<boolean>) {
  const state: Record<string, any> = {
    useCallback: (value: unknown) => value,
    showModal: true, editing: { id: 'instance' }, actionLoading: null,
    formCodexQuickConfig: { experimental_model_catalog_available: true },
    formModelManagementSession: { current: 1 },
    formExperimentalModelCatalogEnabled: false, formModelRoutingEnabled: true,
    confirmDialog: confirmed, t: (key: string) => key, error: 'prior error',
    setFormCodexQuickConfigError: (value: unknown) => { state.error = value; },
    setFormExperimentalModelCatalogEnabled: (value: boolean) => { state.formExperimentalModelCatalogEnabled = value; },
  };
  vm.runInNewContext(compiled, state);
  return state;
}

test('mixed-routing instance models remain read-only until catalog activation is confirmed', async () => {
  for (const accept of [false, true]) {
    let confirmations = 0;
    const h = harness(async () => { confirmations += 1; return accept; });
    assert.equal(h.editorDisabled(), true);
    await h.enable(true);
    assert.equal(confirmations, 1);
    assert.equal(h.formExperimentalModelCatalogEnabled, accept);
    assert.equal(h.editorDisabled(), !accept);
    assert.equal(h.error, accept ? null : 'prior error');
    const model = { model_id: 'relay/model', context_window: 200000, auto_compact_token_limit: 180000 };
    const payload = resolveRoutingCatalog([model], h.formExperimentalModelCatalogEnabled, model.model_id);
    assert.equal(payload.enabled, accept);
    assert.equal(payload.models[0].context_window, 200000);
    assert.equal(payload.models[0].auto_compact_token_limit, 180000);
  }
});

test('an instance catalog confirmation cannot change another form session', async () => {
  const confirmation = deferred<boolean>();
  const h = harness(() => confirmation.promise);
  const pending = h.enable(true);
  h.formModelManagementSession.current += 1;
  confirmation.resolve(true);
  await pending;
  assert.equal(h.formExperimentalModelCatalogEnabled, false);
  assert.equal(h.error, 'prior error');
});

test('catalog disabling needs no confirmation and unavailable or saving instances cannot opt in', async () => {
  const h = harness(async () => { throw new Error('confirmation must not open'); });
  h.formExperimentalModelCatalogEnabled = true;
  await h.enable(false);
  assert.equal(h.formExperimentalModelCatalogEnabled, false);
  assert.equal(h.editorDisabled(), true);
  h.formCodexQuickConfig.experimental_model_catalog_available = false;
  await h.enable(true);
  h.formCodexQuickConfig.experimental_model_catalog_available = true;
  h.actionLoading = 'instance';
  await h.enable(true);
  assert.equal(h.formExperimentalModelCatalogEnabled, false);
});
