// Audit coverage addendum: OpenCode Go per-window strictness and error
// isolation. Complements tests/openCodeGoPlatformMatrix.test.ts without
// touching it. Run with:
//
//   node --test tests/openCodeGoWindowStrictness.test.ts
//
// Invariant under test: a quota snapshot must carry exactly three window
// objects (rolling 5h / weekly / monthly), matching the backend tolerant
// contract (`OpenCodeGoQuotaWindowSnapshot`: status + nullable numerics +
// optional error). Partial or malformed windows degrade per-window to
// status "error"/"unavailable" with null numerics — never fabricated
// zeros — while the connection itself stays listed.
import assert from 'node:assert/strict';
import test from 'node:test';

import {
  normalizeOpenCodeGoConnection,
  normalizeOpenCodeGoQuotaSnapshot,
  toOpenCodeGoCommandError,
} from '../src/services/openCodeGoConnectionService.ts';

function fullWindow(usedPercent = 25) {
  return { usedPercent, remainingPercent: 100 - usedPercent, resetsAt: 1_900_000_000 };
}

function snapshotBase() {
  return {
    rolling: fullWindow(),
    weekly: fullWindow(40),
    monthly: fullWindow(90),
    status: 'available',
    queriedAt: 1_800_000_000,
  };
}

const WINDOW_KEYS = ['rolling', 'weekly', 'monthly'] as const;

test('invariant: exactly three windows survive normalization, each carrying the full window shape', () => {
  const quota = normalizeOpenCodeGoQuotaSnapshot(snapshotBase());
  assert.deepEqual(Object.keys(quota).sort(), ['monthly', 'queriedAt', 'rolling', 'status', 'weekly']);
  for (const key of WINDOW_KEYS) {
    assert.deepEqual(Object.keys(quota[key]).sort(), ['remainingPercent', 'resetsAt', 'status', 'usedPercent']);
    assert.equal(quota[key].status, 'available');
    assert.equal(quota[key].error, undefined);
    assert.equal(
      Math.abs((quota[key].usedPercent ?? Number.NaN) + (quota[key].remainingPercent ?? Number.NaN) - 100) < 1e-9,
      true,
      `${key}: used + remaining must sum to exactly 100%`,
    );
  }
});

test('per-window partial state: a degraded window never fabricates numbers', () => {
  // Tolerant contract: a missing window degrades that window only; the other
  // windows and connection identity survive intact.
  const broken = snapshotBase();
  delete (broken.rolling as Record<string, unknown>).usedPercent;
  delete (broken.rolling as Record<string, unknown>).remainingPercent;
  delete (broken.rolling as Record<string, unknown>).resetsAt;
  broken.status = 'partial';
  const quota = normalizeOpenCodeGoQuotaSnapshot(broken);
  assert.notEqual(quota.rolling.status, 'available');
  assert.equal(quota.rolling.usedPercent, null);
  assert.equal(quota.rolling.remainingPercent, null);
  assert.equal(quota.weekly.usedPercent, 40);
  assert.equal(quota.monthly.remainingPercent, 10);
});

test('per-window partial state: out-of-range values are rejected at the boundary', () => {
  const over = snapshotBase();
  over.monthly.usedPercent = 100.5;
  assert.throws(() => normalizeOpenCodeGoQuotaSnapshot(over), /OPENCODE_GO_INVALID_QUOTA_WINDOW/);

  const negative = snapshotBase();
  negative.weekly.remainingPercent = -1;
  assert.throws(() => normalizeOpenCodeGoQuotaSnapshot(negative), /OPENCODE_GO_INVALID_QUOTA_WINDOW/);

  const past = snapshotBase();
  past.rolling.resetsAt = 0;
  assert.throws(() => normalizeOpenCodeGoQuotaSnapshot(past), /OPENCODE_GO_INVALID_QUOTA_WINDOW/);
});

test('per-window failure isolation: a connection keeps its identity when its quota errors', () => {
  const connection = normalizeOpenCodeGoConnection({
    id: 'conn-1',
    name: 'Primary',
    keyHint: 'oc****01',
    createdAt: 1,
    updatedAt: 2,
    quotaError: { kind: 'NETWORK', occurredAt: 3 },
  });
  assert.equal(connection.id, 'conn-1');
  assert.equal(connection.quota, undefined);
  assert.deepEqual(connection.quotaError, { kind: 'network', occurredAt: 3 });
});

test('error classification maps command failures to stable kinds without raw leakage', () => {
  assert.equal(toOpenCodeGoCommandError('AUTHENTICATION_FAILED'), 'authentication');
  assert.equal(toOpenCodeGoCommandError('RATE_LIMITED'), 'rate_limit');
  assert.equal(toOpenCodeGoCommandError('NETWORK_TIMEOUT'), 'network');
  assert.equal(toOpenCodeGoCommandError('SOMETHING_ELSE'), 'unavailable');
});
