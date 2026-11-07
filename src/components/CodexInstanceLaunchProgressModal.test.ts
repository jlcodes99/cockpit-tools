import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
import { codexLaunchExcludedPids, isCodexLaunchUnconfirmed } from '../utils/codexLaunchConfirmation.ts';
import { codexLaunchErrorKey } from '../utils/codexLaunchProgress.ts';

// Execute the production event reducer without opening an application or dialog.
const source = ts.createSourceFile('modal.tsx', readFileSync(new URL('./CodexInstanceLaunchProgressModal.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let reducer = '';
let factory = '';
ts.forEachChild(source, function visit(node) {
  if (ts.isFunctionDeclaration(node) && node.name?.text === 'createState') factory = node.getText(source);
  if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'applyPayload') reducer = node.initializer!.getText(source);
  ts.forEachChild(node, visit);
});
assert.ok(reducer && factory);
const compiled = ts.transpileModule(`${factory}\nglobalThis.applyPayload = ${reducer};`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

function harness() {
  let state: any = null;
  let actionError: unknown = 'old action error';
  const context: Record<string, any> = {
    disposed: false, STEP_IDS: ['checkInstance', 'writeProfile', 'startClient'],
    setState(update: (previous: unknown) => unknown) { state = update(state); },
    setActionError(value: unknown) { actionError = value; }, setRetryBusy() {},
    proxyEnginePrerequisiteKey: () => null, presentProxyEnginePrerequisite() {},
    codexLaunchErrorKey, codexLaunchExcludedPids, isCodexLaunchUnconfirmed,
    parseCodexSwitchAuthFailure: () => null, t: (key: string) => key,
  };
  vm.runInNewContext(compiled, context);
  const send = (payload: Record<string, unknown>) => context.applyPayload({ instanceId: '__default__', ...payload });
  return { send, state: () => state, error: () => actionError };
}

test('uncertain confirmation becomes a warning and keeps the launch baseline', () => {
  const h = harness();
  h.send({ type: 'start' });
  h.send({ step: 'startClient', stepStatus: 'running', progress: 84 });
  h.send({ type: 'error', error: 'CODEX_START_CONFIRMATION_UNCERTAIN:{"beforePids":[19]}' });
  assert.equal(h.state().status, 'unconfirmed');
  assert.equal(h.state().error, undefined);
  assert.equal(h.state().steps.find((step: any) => step.id === 'startClient').status, 'warning');
  assert.equal(h.state().excludedPids[0], 19);
});

test('completion clears a previous error and its failed start step', () => {
  const h = harness();
  h.send({ type: 'start' });
  h.send({ step: 'startClient', stepStatus: 'running' });
  h.send({ type: 'error', error: 'explicit launch failure' });
  assert.equal(h.state().status, 'error');
  h.send({ type: 'complete' });
  assert.equal(h.state().status, 'completed');
  assert.equal(h.state().error, undefined);
  assert.equal(h.error(), null);
  assert.equal(h.state().steps.find((step: any) => step.id === 'startClient').status, 'completed');
  assert.equal(h.state().steps.find((step: any) => step.id === 'startClient').details.error, undefined);
});

test('retrying the same account starts clean instead of carrying failed steps forward', () => {
  const h = harness();
  h.send({ type: 'start', operation: 'switch-and-start', accountId: 'a' });
  h.send({ step: 'startClient', stepStatus: 'running' });
  h.send({ type: 'error', error: 'explicit launch failure' });
  h.send({ type: 'start', operation: 'switch-and-start', accountId: 'a' });
  assert.equal(h.state().status, 'running');
  assert.equal(h.state().error, undefined);
  assert.equal(h.state().steps.find((step: any) => step.id === 'startClient').status, 'pending');
});
