import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { readFileSync } from 'node:fs';
import { deferred } from '../../../tests/helpers/reactHookHarness';
const source = ts.createSourceFile('fetch.tsx', readFileSync(new URL('./CodexProviderModelFetch.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
function harness() {
  let declaration: ts.VariableDeclaration | undefined;
  const visit = (node: ts.Node) => { if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'fetchModels') declaration = node; ts.forEachChild(node, visit); }; visit(source);
  assert.ok(declaration);
  const task = deferred<{ models: { id: string }[] }>();
  const state: Record<string, any> = {
    baseUrl: 'https://relay.invalid/v1', apiKey: 'fixture-key', wireApi: 'responses', authScheme: 'bearer', signature: 'current',
    current: { current: { signature: 'current', request: 0, mounted: true } },
    setLoading(value: boolean) { state.loading = value; }, setError(value: any) { state.error = value; },
    listModelProviderModels(input: any) { state.request = input; return task.promise; },
    onModels(models: string[]) { state.models = models; }, t: (key: string) => key,
  };
  vm.runInNewContext(ts.transpileModule(`globalThis.run = ${declaration.initializer!.getText(source)}`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, state);
  return { state, task };
}
test('shared model discovery forwards protocol and authentication and fills only the current draft', async () => {
  const { state, task } = harness(); const pending = state.run(); task.resolve({ models: [{ id: 'a' }, { id: 'a' }, { id: 'b' }] }); await pending;
  assert.equal(state.request.wireApi, 'responses'); assert.equal(state.request.authScheme, 'bearer');
  assert.deepEqual(Array.from(state.models), ['a', 'b']); assert.equal(state.loading, false);
});
for (const change of ['configuration', 'close']) test(`late model discovery cannot write after ${change}`, async () => {
  const { state, task } = harness(); const pending = state.run();
  if (change === 'close') state.current.current.mounted = false; else state.current.current.signature = 'different-provider-or-protocol';
  task.resolve({ models: [{ id: 'obsolete' }] }); await pending;
  assert.equal(state.models, undefined);
});
