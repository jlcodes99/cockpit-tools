import assert from 'node:assert/strict';
import test from 'node:test';
import {
  OPENCODE_GO_CONNECTION_LIMIT,
  normalizeOpenCodeGoConnection,
  normalizeOpenCodeGoQuotaSnapshot,
  toOpenCodeGoCommandError,
} from './openCodeGoConnectionService.ts';

test('OpenCode Go connection summaries contain hints but never credential material', () => {
  const summary = normalizeOpenCodeGoConnection({
    id: 'ocg_1',
    name: ' Primary ',
    keyHint: 'ocg-****-key',
    apiKey: 'must-not-escape',
    createdAt: 10,
    updatedAt: 20,
  });
  assert.deepEqual(summary, {
    id: 'ocg_1',
    name: 'Primary',
    keyHint: 'ocg-****-key',
    createdAt: 10,
    updatedAt: 20,
    enabled: true,
    provider: 'go',
  });
  assert.equal(JSON.stringify(summary).includes('must-not-escape'), false);
});

test('OpenCode Go quota normalization preserves three finite windows', () => {
  const quota = normalizeOpenCodeGoQuotaSnapshot({
    rolling: { usedPercent: 25, remainingPercent: 75, resetsAt: 1_900_000_000 },
    weekly: { usedPercent: 40, remainingPercent: 60, resetsAt: 1_900_100_000 },
    monthly: { usedPercent: 90, remainingPercent: 10, resetsAt: 1_900_200_000 },
    status: 'available',
    queriedAt: 1_800_000_000,
    responseBody: 'must-not-escape',
  });
  assert.equal(quota.rolling.status, 'available');
  assert.equal(quota.rolling.remainingPercent, 75);
  assert.equal(quota.weekly.resetsAt, 1_900_100_000);
  assert.equal(quota.monthly.usedPercent, 90);
  assert.equal(JSON.stringify(quota).includes('must-not-escape'), false);
});

test('OpenCode Go quota normalization preserves partial windows independently', () => {
  const quota = normalizeOpenCodeGoQuotaSnapshot({
    rolling: {
      status: 'available',
      usedPercent: 25,
      remainingPercent: 75,
      resetsAt: 1_900_000_000,
    },
    weekly: {
      status: 'available',
      usedPercent: 60,
      remainingPercent: 40,
      resetsAt: 1_900_100_000,
    },
    monthly: {
      status: 'error',
      usedPercent: null,
      remainingPercent: null,
      resetsAt: null,
      error: 'window unavailable',
      responseSecret: 'must-not-escape',
    },
    status: 'partial',
    queriedAt: 1_800_000_000,
  });

  assert.equal(quota.rolling.remainingPercent, 75);
  assert.equal(quota.weekly.remainingPercent, 40);
  assert.deepEqual(quota.monthly, {
    status: 'error',
    usedPercent: null,
    remainingPercent: null,
    resetsAt: null,
    error: 'window unavailable',
  });
  assert.equal(JSON.stringify(quota).includes('must-not-escape'), false);
});

test('OpenCode Go quota normalization tolerates missing window fields', () => {
  const quota = normalizeOpenCodeGoQuotaSnapshot({
    rolling: { status: 'error', usedPercent: 10, error: 'reset missing' },
    weekly: { status: 'error', resetsAt: 1_900_100_000, error: 'percent invalid' },
    monthly: { status: 'unavailable', error: 'window missing' },
    status: 'partial',
    queriedAt: 1_800_000_000,
  });

  assert.deepEqual(quota.rolling, {
    status: 'error',
    usedPercent: 10,
    remainingPercent: null,
    resetsAt: null,
    error: 'reset missing',
  });
  assert.equal(quota.weekly.resetsAt, 1_900_100_000);
  assert.equal(quota.monthly.status, 'unavailable');
});

test('OpenCode Go command errors expose only stable classifications', () => {
  assert.equal(toOpenCodeGoCommandError('OPENCODE_GO_USAGE_AUTHENTICATION: token=secret'), 'authentication');
  assert.equal(toOpenCodeGoCommandError('OPENCODE_GO_USAGE_RATE_LIMIT'), 'rate_limit');
  assert.equal(toOpenCodeGoCommandError(new Error('socket secret details')), 'unavailable');
  assert.equal(OPENCODE_GO_CONNECTION_LIMIT, 4);
});
