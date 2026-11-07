import assert from 'node:assert/strict';
import test from 'node:test';
import { codexLaunchErrorKey, mapCodexSwitchProgressToLaunch } from './codexLaunchProgress.ts';

test('lifecycle and proxy failures remain distinct from authorization errors', () => {
  assert.equal(codexLaunchErrorKey('CODEX_SWITCH_BINDING_FAILED'), 'codex.launchErrors.bindingFailed');
  assert.equal(codexLaunchErrorKey(new Error('CODEX_PROCESS_SCAN_FAILED')), 'codex.launchErrors.processScanFailed');
  assert.equal(codexLaunchErrorKey('PROXY_ENTRY_PORT_UNAVAILABLE'), 'codex.launchErrors.proxyEntryFailed');
  assert.equal(codexLaunchErrorKey('CODEX_SWITCH_AUTH_REQUIRED:{}'), null);
  assert.equal(codexLaunchErrorKey('unknown error or private diagnostic data'), null);
});

test('client proxy setup failures explain the route problem instead of requesting reauthorization', () => {
  assert.equal(codexLaunchErrorKey('CLIENT_PROXY_BOUND_OAUTH_MISSING'), 'codex.proxy.boundAccountUnavailable');
  assert.equal(codexLaunchErrorKey(new Error('CLIENT_PROXY_BOUND_PROXY_MISSING')), 'codex.proxy.boundProxyUnavailable');
  assert.equal(codexLaunchErrorKey('CLIENT_PROXY_STALE'), 'codex.proxy.sourceUnavailable');
  assert.equal(codexLaunchErrorKey('CLIENT_PROXY_STORAGE'), 'codex.proxy.runtimeReadFailed');
  assert.equal(codexLaunchErrorKey('CLIENT_PROXY_TIMEOUT'), 'codex.proxy.operationTimeout');
  assert.equal(codexLaunchErrorKey('CLIENT_PROXY_FAILED'), 'codex.proxy.saveFailed');
  assert.equal(codexLaunchErrorKey('CLIENT_PROXY_STALE https://user:secret@private.invalid'), null);
  assert.equal(codexLaunchErrorKey('toString'), null);
});

test('bounded session checks keep contextual diagnostics out of both launch dialogs', () => {
  assert.equal(codexLaunchErrorKey('CODEX_SESSION_VISIBILITY_TIMEOUT'), 'codex.launchErrors.sessionCheckTimeout');
  assert.equal(codexLaunchErrorKey(new Error('Session repair (default / private-path): CODEX_SESSION_VISIBILITY_TIMEOUT: sqlite')), 'codex.launchErrors.sessionCheckTimeout');
  assert.equal(codexLaunchErrorKey('Session repair: CODEX_SESSION_VISIBILITY_ROLLOUT_LINE_TOO_LARGE: private-path'), 'codex.launchErrors.sessionFileTooLarge');
  assert.equal(codexLaunchErrorKey('CODEX_SESSION_VISIBILITY_ROLLBACK_FAILED: CODEX_SESSION_VISIBILITY_TIMEOUT; backup: private-path'), 'codex.launchErrors.sessionRestoreFailed');
  assert.equal(codexLaunchErrorKey('CODEX_SESSION_VISIBILITY_TIMEOUT_OTHER'), null);
  assert.equal(codexLaunchErrorKey('CODEX_SESSION_VISIBILITY_ROLLOUT_LINE_TOO_LARGE_OTHER'), null);
});

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

test('preserves id_token details and normalizes refresh progress into the shared launch protocol', () => {
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
    expiresAt: 1_800_003_600,
    refreshDue: true,
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
