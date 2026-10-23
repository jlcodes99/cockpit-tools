import assert from 'node:assert/strict';
import test from 'node:test';
import type { TFunction } from 'i18next';
import { formatAntigravitySwitchError } from './antigravitySwitchError.ts';

test('desktop switch errors use localized keys while path recovery remains intact', () => {
  const t = ((key: string, options?: { error: string }) => `localized:${key}${options ? ':' + options.error : ''}`) as TFunction;
  assert.equal(formatAntigravitySwitchError(t, new Error('ANTIGRAVITY_SWITCH_BUSY')), 'localized:messages.antigravitySwitchBusy');
  assert.equal(formatAntigravitySwitchError(t, 'ANTIGRAVITY_SWITCH_PREPARATION_TIMEOUT'), 'localized:messages.antigravitySwitchPrepareTimeout');
  assert.equal(formatAntigravitySwitchError(t, 'APP_PATH_NOT_FOUND:antigravity'), 'APP_PATH_NOT_FOUND:antigravity');
  assert.equal(formatAntigravitySwitchError(t, 'provider failed'), 'provider failed');
  assert.equal(formatAntigravitySwitchError(t, 'ANTIGRAVITY_SWITCH_STATE_SAVE_FAILED: disk full'), 'localized:messages.antigravitySwitchStateSaveFailed:disk full');
});
