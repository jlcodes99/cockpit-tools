import assert from 'node:assert/strict';
import test from 'node:test';
import type { TFunction } from 'i18next';
import { resolveUpdateApplyFailureMessage } from './updateApplyFailureMessage.ts';

const t = ((key: string) => `localized:${key}`) as TFunction;

test('install failure while translocated uses the translocation hint', () => {
  assert.equal(
    resolveUpdateApplyFailureMessage(t, 'install', true),
    'localized:update_notification.installFailedTranslocated',
  );
});

test('install failure outside translocation keeps the generic message', () => {
  assert.equal(
    resolveUpdateApplyFailureMessage(t, 'install', false),
    'localized:update_notification.installFailed',
  );
});

test('other stages ignore translocation', () => {
  for (const translocated of [false, true]) {
    assert.equal(
      resolveUpdateApplyFailureMessage(t, 'prepare', translocated),
      'localized:update_notification.stopApiServiceFailed',
    );
    assert.equal(
      resolveUpdateApplyFailureMessage(t, 'relaunch', translocated),
      'localized:update_notification.restartRequiredAfterInstall',
    );
  }
});
