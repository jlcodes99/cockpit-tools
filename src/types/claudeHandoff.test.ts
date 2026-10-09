import assert from 'node:assert/strict';
import test from 'node:test';
import {
  canPreviewClaudeHandoff,
  canApplyClaudeHandoffPreview,
  canRollbackClaudeHandoff,
  getClaudeHandoffErrorCode,
  getClaudeHandoffProcessDiagnostic,
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

test('process diagnostics accept only bounded metadata and discard raw backend details', () => {
  const valid = { code: 'DESKTOP_UPDATE_TIMEOUT', processId: 4242, processRole: 'desktop-updater', processName: 'ShipIt' };
  const expected = { processId: 4242, processRole: 'desktop-updater', processName: 'ShipIt' };
  assert.deepEqual(getClaudeHandoffProcessDiagnostic(JSON.stringify({ ...valid, message: 'ignored', args: '--ignored' })), expected);
  for (const code of ['CLAUDE_WRITER_RUNNING', 'DESKTOP_UPDATE_TIMEOUT', 'DESKTOP_CONTRACT_CHANGED']) {
    const payload = JSON.stringify({ ...valid, code, args: '--ignored', executable: '/ignored/path' });
    for (const error of [payload, `Error: ${payload}`, { message: payload }, new Error(payload), { message: `Error: ${payload}` }]) {
      assert.equal(getClaudeHandoffErrorCode(error), code);
      assert.deepEqual(getClaudeHandoffProcessDiagnostic(error), expected);
    }
  }
  for (const processRole of ['desktop-main', 'claude-cli', 'desktop-helper', 'desktop-updater']) {
    assert.deepEqual(getClaudeHandoffProcessDiagnostic({ ...valid, code: 'CLAUDE_WRITER_RUNNING', processRole }), { ...expected, processRole });
  }
  for (const processName of ['/private/ShipIt', 'C:\\ShipIt', 'ShipIt --token secret', 'ShipIt\nsecret', '..', 'x'.repeat(129), ' ShipIt', 'ShipIt\u202esecret']) {
    assert.equal(getClaudeHandoffProcessDiagnostic({ ...valid, processName }), null);
  }
  for (const processId of [0, -1, 1.5, NaN, Infinity, 0x100000000, '4242', null]) {
    assert.equal(getClaudeHandoffProcessDiagnostic({ ...valid, processId }), null);
  }
  for (const value of [null, [], 'CLAUDE_WRITER_RUNNING: pid=4242', '{bad', { ...valid, code: 'UNKNOWN' }, { ...valid, processRole: 'unknown' }, { ...valid, processName: null }]) {
    assert.equal(getClaudeHandoffProcessDiagnostic(value), null);
  }
  for (const processName of ['ShipIt -x secret', 'ShipIt --flag=value', 'ShipIt; secret', 'ShipIt\tsecret', 'ShipIt\u0000secret']) {
    const wrapper = { message: `Error: ${JSON.stringify({ ...valid, code: 'DESKTOP_CONTRACT_CHANGED', processName })}` };
    assert.equal(getClaudeHandoffProcessDiagnostic(wrapper), null);
  }
  let nested: unknown = valid;
  for (let i = 0; i < 10; i += 1) nested = { message: JSON.stringify(nested) };
  for (const error of [nested, 'Error: {' + ' '.repeat(16_384) + '}']) {
    assert.equal(getClaudeHandoffProcessDiagnostic(error), null);
    assert.equal(getClaudeHandoffErrorCode(error), null);
  }
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
  assert.equal(getClaudeHandoffErrorCode('CLAUDE_WRITER_RUNNING: pid=4242 role=shipit executable=ShipIt'), 'CLAUDE_WRITER_RUNNING');
  assert.equal(getClaudeHandoffErrorCode({ code: 'DESKTOP_UPDATE_TIMEOUT' }), 'DESKTOP_UPDATE_TIMEOUT');
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
