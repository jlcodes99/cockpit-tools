import assert from 'node:assert/strict';
import test from 'node:test';
import { mapCodexSwitchProgressToLaunch } from './codexLaunchProgress.ts';

test('maps account overview access_token progress to shared launch fields', () => {
  const result = mapCodexSwitchProgressToLaunch({
    accountId: 'account-1',
    step: 'accessToken',
    stepStatus: 'completed',
    details: {
      expiresAt: 1_800_000_000,
      refreshDue: false,
    },
  });

  assert.equal(result?.step, 'checkAccount');
  assert.deepEqual(result?.details, {
    accountId: 'account-1',
    accessTokenExpiresAt: 1_800_000_000,
    accessTokenRefreshDue: false,
  });
});

test('merges id_token and refresh progress into the shared launch protocol', () => {
  const idToken = mapCodexSwitchProgressToLaunch({
    accountId: 'account-1',
    step: 'idToken',
    details: { expiresAt: 1_800_003_600, refreshDue: true },
  });
  const refresh = mapCodexSwitchProgressToLaunch({
    accountId: 'account-1',
    step: 'refreshTokens',
    details: {
      required: true,
      tokenGenerationChanged: true,
      accessTokenExpiresAt: 1_800_010_000,
      idTokenExpiresAt: 1_800_006_000,
    },
  });

  assert.deepEqual(idToken?.details, {
    accountId: 'account-1',
    idTokenExpiresAt: 1_800_003_600,
    idTokenRefreshDue: true,
  });
  assert.deepEqual(refresh?.details, {
    accountId: 'account-1',
    refreshRequired: true,
    tokenGenerationChanged: true,
    accessTokenExpiresAt: 1_800_010_000,
    idTokenExpiresAt: 1_800_006_000,
  });
});

test('rejects switch events without a target account', () => {
  assert.equal(mapCodexSwitchProgressToLaunch({ step: 'accessToken' }), null);
});
