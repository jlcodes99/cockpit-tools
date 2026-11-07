import assert from 'node:assert/strict';
import test from 'node:test';
import { summarizeCodexInstanceGateways } from './codexInstanceGatewaySummary.ts';

test('an explicitly stopped or not-yet-started gateway is not an issue', () => {
  assert.deepEqual(summarizeCodexInstanceGateways([
    { status: 'running', lastError: null },
    { status: 'stopped', lastError: null },
  ]), { total: 2, running: 1, issues: 0 });
  assert.deepEqual(summarizeCodexInstanceGateways([
    { status: 'stopped', lastError: '  ' },
    { status: 'notStarted', lastError: null },
  ]), { total: 2, running: 0, issues: 0 });
});

test('connectivity failures and retained recovery errors remain issues', () => {
  assert.deepEqual(summarizeCodexInstanceGateways([
    { status: 'unreachable', lastError: null },
    { status: 'portConflict', lastError: null },
    { status: 'stopped', lastError: 'Failed to restart sidecar' },
    { status: 'notStarted', lastError: 'Failed to prepare profile' },
  ]), { total: 4, running: 0, issues: 4 });
});
