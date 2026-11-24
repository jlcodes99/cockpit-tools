import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexLaunchStepStatus } from './codexLaunchProgress';
import { applyCountryProgress } from './codexCountryProgress';
import type { CodexCountryCheck } from '../services/codexProxyCountryService';

const report: CodexCountryCheck = { checkId: 'fixture', accountId: 'a', scope: 'launch',
  expectedCountry: 'JP', actualCountry: null, status: 'checking', checkedAt: 1,
  elapsedMs: 0, source: 'account', reason: null, stale: false };
const initial = (): { accountId: string; status: string;
  steps: { id: string; status: CodexLaunchStepStatus; details: Record<string, unknown> }[] } => ({ accountId: '__provider_gateway__:a', status: 'running', steps: [
  { id: 'checkProxyCountry', status: 'skipped' as const, details: {} as Record<string, unknown> },
  { id: 'startClient', status: 'pending' as const, details: {} as Record<string, unknown> },
] });
test('country checks have their own step and leave client startup pending on failure', () => {
  const checking = applyCountryProgress(initial(), report)!;
  assert.equal(checking.steps[0].status, 'running');
  const failed = applyCountryProgress(checking, { ...report, status: 'blocked', reason: 'PROXY_COUNTRY_MISMATCH:JP:SG', actualCountry: 'SG' })!;
  assert.equal(failed.steps[0].status, 'error');
  assert.equal(failed.steps[1].status, 'pending');
  assert.equal(failed.steps[0].details.actualCountry, 'SG');
  const retried = applyCountryProgress(initial(), { ...report, checkId: 'retry' })!;
  const passed = applyCountryProgress(retried, { ...report, checkId: 'retry', status: 'passed', actualCountry: 'JP' })!;
  assert.equal(passed.steps[0].status, 'completed');
  assert.equal(passed.steps[0].details.error, undefined);
});
test('manual, unrelated and late country events cannot open or rewrite a launch', () => {
  const state = initial();
  assert.equal(applyCountryProgress(null, report), null);
  assert.equal(applyCountryProgress(state, { ...report, accountId: 'b' }), state);
  assert.equal(applyCountryProgress(state, { ...report, scope: 'manual' }), state);
  const complete = { ...state, status: 'completed' };
  assert.equal(applyCountryProgress(complete, report), complete);
});
