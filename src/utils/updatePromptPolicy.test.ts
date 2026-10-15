import assert from 'node:assert/strict';
import test from 'node:test';
import { UpdatePromptPolicy } from './updatePromptPolicy';

test('automatic prompts wait for local preference and cannot override disabled reminders', () => {
  const policy = new UpdatePromptPolicy();
  assert.equal(policy.openAutomatic('1.4.0', 'popup'), false);
  policy.applyPreference(false);
  assert.equal(policy.openAutomatic('1.4.0', 'popup'), false);
  policy.applyPreference(true);
  assert.equal(policy.openAutomatic('1.4.0', 'silent'), false);
  assert.equal(policy.openAutomatic('1.4.0', 'popup'), true);
  policy.close();
  assert.equal(policy.openAutomatic('1.4.0', 'popup'), false);
});

test('late settings responses cannot overwrite a preference changed while checking', async () => {
  const policy = new UpdatePromptPolicy();
  const revision = policy.revision;
  let resolveSettings!: (enabled: boolean) => void;
  const settings = new Promise<boolean>((resolve) => { resolveSettings = resolve; });
  const pendingCheck = settings.then((enabled) => {
    policy.applyPreference(enabled, revision);
    return policy.openAutomatic('1.4.0', 'popup');
  });
  policy.applyPreference(false);
  resolveSettings(true);
  assert.equal(await pendingCheck, false);
  assert.equal(policy.enabled, false);
});

test('disabling during a download suppresses later automatic prompts and closes only auto UI', () => {
  const policy = new UpdatePromptPolicy();
  policy.applyPreference(true);
  assert.equal(policy.openAutomatic('1.4.0', 'popup'), true);
  policy.applyPreference(false);
  assert.equal(policy.shouldCloseAutomaticPrompt(), true);
  policy.close();
  assert.equal(policy.openAutomatic('1.4.1', 'popup'), false);
});

test('manual check and details remain usable with reminders off and cannot become automatic', () => {
  const policy = new UpdatePromptPolicy();
  policy.openManual();
  policy.applyPreference(false);
  assert.equal(policy.shouldCloseAutomaticPrompt(), false);
  policy.applyPreference(true);
  assert.equal(policy.openAutomatic('1.4.0', 'popup'), false);
  policy.applyPreference(false);
  assert.equal(policy.shouldCloseAutomaticPrompt(), false);
  policy.close();
  policy.applyPreference(true);
  assert.equal(policy.openAutomatic('1.4.0', 'popup'), true);
});

test('a stale disabled snapshot cannot undo re-enabling reminders', () => {
  const policy = new UpdatePromptPolicy();
  policy.applyPreference(false);
  const revision = policy.revision;
  policy.applyPreference(true);
  assert.equal(policy.applyPreference(false, revision), false);
  assert.equal(policy.openAutomatic('1.4.0', 'popup'), true);
});
