import assert from 'node:assert/strict';
import test from 'node:test';
import {
  canPreviewClaudeHandoff,
  canApplyClaudeHandoffPreview,
  canRollbackClaudeHandoff,
  getClaudeHandoffErrorCode,
  getClaudeHandoffRunAction,
  type ClaudeHandoffStatus,
  type ClaudeHandoffPreview,
} from './claudeHandoff.ts';

const ready: ClaudeHandoffStatus = {
  supported: true,
  reason: null,
  desktopVersion: '1.0.fixture',
  accounts: [
    { id: 'source', label: 'Source', eligible: true, reason: null },
    { id: 'target', label: 'Target', eligible: true, reason: null },
    { id: 'uninitialized', label: 'Uninitialized', eligible: false, reason: 'SIDEBAR_NOT_INITIALIZED' },
  ],
  runs: [],
};
const pair = { sourceAccountId: 'source', targetAccountId: 'target' };

test('equal conversations can remember shared state without rewriting records', () => {
  const preview: ClaudeHandoffPreview = {
    fingerprint: 'reviewed', desktopVersion: '1.0.fixture', created: 0, updated: 0,
    unchanged: 3, missing: 0, stale: 0, replacedBranches: 0, baselineChanged: true, issues: [], warnings: [], quotaPausesCleared: 0,
  };
  assert.equal(canApplyClaudeHandoffPreview(preview), true);
  assert.equal(canApplyClaudeHandoffPreview({ ...preview, baselineChanged: false }), false);
  assert.equal(canApplyClaudeHandoffPreview({ ...preview, baselineChanged: false, created: 1 }), true);
  assert.equal(canApplyClaudeHandoffPreview({ ...preview, baselineChanged: false, updated: 1 }), true);
  assert.equal(canApplyClaudeHandoffPreview({ ...preview, fingerprint: '' }), false);
  for (const desktopVersion of ['', 'unknown', '999.0.future']) {
    assert.equal(canApplyClaudeHandoffPreview({ ...preview, desktopVersion }), true,
      'diagnostic version must not block an otherwise applicable preview');
    assert.equal(canApplyClaudeHandoffPreview({ ...preview, desktopVersion, missing: 1 }), false);
    assert.equal(canApplyClaudeHandoffPreview({ ...preview, desktopVersion, stale: 1 }), false);
    assert.equal(canApplyClaudeHandoffPreview({ ...preview, desktopVersion, replacedBranches: 1 }), false);
  }
  assert.equal(canApplyClaudeHandoffPreview({ ...preview, created: 1, missing: 1 }), false);
  assert.equal(canApplyClaudeHandoffPreview({ ...preview, created: 1, stale: 1 }), false);
  assert.equal(canApplyClaudeHandoffPreview({ ...preview, created: 1, replacedBranches: 1 }), false);
  assert.equal(canApplyClaudeHandoffPreview(null), false);
});

test('supported storage permits preview regardless of the diagnostic Desktop version', () => {
  assert.equal(canPreviewClaudeHandoff(ready, pair), true);
  for (const desktopVersion of [null, '', 'unknown', '999.0.future']) {
    assert.equal(canPreviewClaudeHandoff({ ...ready, desktopVersion }, pair), true);
  }
});

test('handoff preview fails closed for unsupported storage and invalid identities', () => {
  for (const status of [null, { ...ready, supported: false }]) {
    assert.equal(canPreviewClaudeHandoff(status, pair), false);
  }
  for (const targetAccountId of ['', 'source', 'missing', 'uninitialized']) {
    assert.equal(canPreviewClaudeHandoff(ready, { ...pair, targetAccountId }), false);
  }
  assert.equal(canPreviewClaudeHandoff(ready, { ...pair, sourceAccountId: 'uninitialized' }), false);
  assert.equal(canPreviewClaudeHandoff(ready, { ...pair, sourceAccountId: '' }), false);
  assert.equal(canPreviewClaudeHandoff(ready, { ...pair, sourceAccountId: 'missing' }), false);
});

test('only completed rollback hides recovery; unknown and interrupted states stay recoverable', () => {
  assert.equal(getClaudeHandoffRunAction('applied'), 'rollback');
  assert.equal(getClaudeHandoffRunAction('rolled_back'), null);
  for (const state of ['preparing', 'applying', 'pending', 'rolling_back', 'failed', 'new_backend_state']) {
    assert.equal(getClaudeHandoffRunAction(state), 'recover');
  }
});

test('unavailable or incompatible storage blocks new handoffs but permits saved-preimage recovery', () => {
  for (const reason of ['DESKTOP_CONTRACT_UNAVAILABLE', 'DESKTOP_CONTRACT_UNSUPPORTED', 'DESKTOP_CONTRACT_CHANGED', 'DESKTOP_VERSION_REQUIRES_REVIEW']) {
    for (const desktopVersion of [null, '', '999.0.future']) {
      const unsupported = { ...ready, supported: false, reason, desktopVersion };
      assert.equal(canRollbackClaudeHandoff(unsupported), true);
      assert.equal(canPreviewClaudeHandoff(unsupported, pair), false);
    }
  }
  assert.equal(canRollbackClaudeHandoff(ready), true);
  assert.equal(canRollbackClaudeHandoff(null), false);
  for (const reason of [null, 'MACOS_REQUIRED', 'DEFAULT_PROFILE_REQUIRED', 'DESKTOP_VERSION_UNAVAILABLE', 'APP_PATH_NOT_FOUND:claude', 'UNKNOWN']) {
    assert.equal(canRollbackClaudeHandoff({ ...ready, supported: false, reason }), false);
  }
});

test('extracts stable IPC error codes without exposing private backend details', () => {
  assert.equal(getClaudeHandoffErrorCode('PREVIEW_CHANGED'), 'PREVIEW_CHANGED');
  assert.equal(getClaudeHandoffErrorCode('Error: SIDEBAR_NOT_INITIALIZED: /private/account/path'), 'SIDEBAR_NOT_INITIALIZED');
  assert.equal(getClaudeHandoffErrorCode('[ACCOUNT_NOT_FOUND] private-account-id'), 'ACCOUNT_NOT_FOUND');
  assert.equal(getClaudeHandoffErrorCode(new Error('HANDOFF_BUSY')), 'HANDOFF_BUSY');
  assert.equal(getClaudeHandoffErrorCode({ code: 'DESKTOP_VERSION_CHANGED', message: 'Private detail' }), 'DESKTOP_VERSION_CHANGED');
  for (const code of ['DESKTOP_CONTRACT_UNAVAILABLE', 'DESKTOP_CONTRACT_UNSUPPORTED', 'DESKTOP_CONTRACT_CHANGED', 'UNSUPPORTED_PERSISTED_FIELD']) {
    assert.equal(getClaudeHandoffErrorCode({ code, message: '/private/account/storage' }), code);
    assert.equal(getClaudeHandoffErrorCode(JSON.stringify({ code, message: '/private/account/storage' })), code);
    assert.equal(getClaudeHandoffErrorCode(`Error: ${code}: /private/account/storage`), code);
  }
  assert.equal(getClaudeHandoffErrorCode('{"code":"ROLLBACK_CONFLICT","message":"private path"}'), 'ROLLBACK_CONFLICT');
  for (const value of [null, undefined, 42, {}, 'unexpected private path', '{invalid JSON', { code: 'private/path' }]) {
    assert.equal(getClaudeHandoffErrorCode(value), null);
  }
});
