import assert from 'node:assert/strict';
import test from 'node:test';
import type { TFunction } from 'i18next';
import { codexBackgroundError } from './codexBackgroundError';

const t = ((key: string) => `translated:${key}`) as TFunction;

test('background official errors use shared authorization and usage translations', () => {
  assert.equal(codexBackgroundError('CHATGPT_PLAN_REAUTH_REQUIRED', t), 'translated:chatgptPlan.error.reauthorize');
  assert.equal(codexBackgroundError(new Error('CHATGPT_PLAN_USAGE_LIMIT'), t), 'translated:chatgptPlan.error.usage');
  assert.equal(codexBackgroundError('CHATGPT_PLAN_BINDING_CONFLICT', t), 'translated:chatgptPlan.error.bindingConflict');
});

test('background error rendering preserves ordinary and embedded upstream diagnostics', () => {
  for (const message of ['HTTP 503: unavailable', 'message mentions CHATGPT_PLAN_USAGE_LIMIT', 'PELICAN_CANCELLED']) {
    assert.equal(codexBackgroundError(message, t), message);
  }
});
