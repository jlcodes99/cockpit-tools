import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
import { codexLaunchExcludedPids, isCodexLaunchUnconfirmed } from '../utils/codexLaunchConfirmation.ts';
import { codexCountryErrorKey, codexCountryErrorValues } from '../utils/codexProxyCountry';
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
    disposed: false, STEP_IDS: ['checkInstance', 'writeProfile', 'checkProxyCountry', 'startClient'],
    setState(update: (previous: unknown) => unknown) { state = update(state); },
    setActionError(value: unknown) { actionError = value; }, setRetryBusy() {},
    proxyEnginePrerequisiteKey: () => null, presentProxyEnginePrerequisite() {},
    codexCountryErrorKey, codexCountryErrorValues, codexLaunchErrorKey, codexLaunchExcludedPids, isCodexLaunchUnconfirmed,
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

test('successful launch retains project recovery deferral details', () => {
  const h = harness();
  const message = 'Quick project scan reached its limit. Largest state file: 2542520 bytes.';
  h.send({ type: 'start' });
  h.send({
    step: 'writeProfile',
    stepStatus: 'warning',
    details: { sessionMaintenanceDeferred: true, projectRecoveryDeferredMessage: message },
  });
  h.send({ type: 'complete' });
  const step = h.state().steps.find((item: any) => item.id === 'writeProfile');
  assert.equal(h.state().status, 'completed');
  assert.equal(step.status, 'warning');
  assert.equal(step.details.projectRecoveryDeferredMessage, message);
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

test('a committed switch retains its separate CLI resume warning', () => {
  const h = harness();
  h.send({ type: 'start', operation: 'switch-and-start' });
  h.send({
    step: 'startClient',
    stepStatus: 'warning',
    details: { launchMode: 'cli', cliResumeError: 'Profile already owns a CLI process' },
  });
  h.send({ type: 'complete' });
  h.send({ step: 'startClient', stepStatus: 'completed', details: { error: null } });
  h.send({ type: 'complete' });
  const step = h.state().steps.find((item: any) => item.id === 'startClient');
  assert.equal(h.state().status, 'completed');
  assert.equal(step.status, 'warning');
  assert.equal(step.details.cliResumeError, 'Profile already owns a CLI process');
});

test('the launch error retains the expected CLI binding for retry', () => {
  const h = harness();
  h.send({ type: 'start', expectedSwitchBinding: 'fixture-provider' });
  h.send({ type: 'error', error: 'launch failed' });
  assert.equal(h.state().expectedSwitchBinding, 'fixture-provider');
  h.send({ type: 'start' });
  assert.equal(h.state().expectedSwitchBinding, undefined);
  h.send({
    type: 'error',
    error: 'CODEX_SWITCH_BINDING_FAILED',
    expectedSwitchBinding: 'fixture-next',
  });
  assert.equal(h.state().expectedSwitchBinding, 'fixture-next');
});

test('a blocked country check does not label client startup or credentials as failed', () => {
  const h = harness(); h.send({ type: 'start', accountId: 'a' });
  h.send({ step: 'checkInstance', stepStatus: 'running' });
  h.send({ step: 'checkProxyCountry', stepStatus: 'error', details: { error: 'PROXY_COUNTRY_MISMATCH:JP:SG' } });
  h.send({ type: 'error', error: 'PROXY_COUNTRY_MISMATCH:JP:SG' });
  assert.equal(h.state().steps.find((step: any) => step.id === 'checkProxyCountry').status, 'error');
  assert.equal(h.state().steps.find((step: any) => step.id === 'checkInstance').status, 'pending');
  assert.equal(h.state().steps.find((step: any) => step.id === 'startClient').status, 'pending');
});

test('country error codes select their own step even if a live evidence event was missed', () => {
  const h = harness(); h.send({ type: 'start', accountId: 'a' });
  h.send({ step: 'startClient', stepStatus: 'running' });
  h.send({ type: 'error', error: 'PROXY_COUNTRY_UNAVAILABLE' });
  assert.equal(h.state().steps.find((step: any) => step.id === 'checkProxyCountry').status, 'error');
  assert.equal(h.state().steps.find((step: any) => step.id === 'startClient').status, 'pending');
});
