import assert from 'node:assert/strict';
import test from 'node:test';
import { confirmedCodexLaunch, codexLaunchExcludedPids, isCodexLaunchUnconfirmed } from './codexLaunchConfirmation.ts';
import type { InstanceProfile } from '../types/instance';

const running = { id: '__default__', running: true, lastPid: 42, userDataDir: 'C:\\Users\\a\\.codex' } as InstanceProfile;

test('only ambiguous startup confirmation becomes pending; explicit failures stay failures', () => {
  assert.ok(isCodexLaunchUnconfirmed(new Error('CODEX_START_CONFIRMATION_UNCERTAIN: no receipt')));
  assert.ok(isCodexLaunchUnconfirmed('Codex startup confirmation failed: no stable new default process'));
  for (const error of ['CODEX_SWITCH_AUTH_REQUIRED:{}', 'permission denied', 'Codex exited before startup confirmation: 1', 'CODEX_SESSION_VISIBILITY_ROLLBACK_FAILED']) {
    assert.equal(isCodexLaunchUnconfirmed(error), false);
  }
});

test('confirmation requires a live PID and the selected instance/profile, without conflating case-sensitive homes', () => {
  assert.equal(confirmedCodexLaunch([running], '__default__', 'c:/users/a/.codex/'), running);
  assert.equal(confirmedCodexLaunch([running], 'managed'), undefined);
  assert.equal(confirmedCodexLaunch([running], '__default__', 'C:/Users/a/other'), undefined);
  assert.equal(confirmedCodexLaunch([running], '__default__', undefined, [42]), undefined);
  assert.equal(confirmedCodexLaunch([{ ...running, running: false }], '__default__'), undefined);
  assert.equal(confirmedCodexLaunch([{ ...running, lastPid: null }], '__default__'), undefined);
  assert.equal(confirmedCodexLaunch([{ ...running, userDataDir: '/Users/A/.codex' }], '__default__', '/Users/a/.codex'), undefined);
});

test('reconciliation preserves the launch baseline and rejects malformed PID values', () => {
  assert.deepEqual(codexLaunchExcludedPids('CODEX_START_CONFIRMATION_UNCERTAIN:{"beforePids":[42,50,-1,"60",null,1.5]}'), [42, 50]);
  assert.deepEqual(codexLaunchExcludedPids('CODEX_START_CONFIRMATION_UNCERTAIN:missing receipt'), []);
  assert.deepEqual(codexLaunchExcludedPids('permission denied'), []);
});
