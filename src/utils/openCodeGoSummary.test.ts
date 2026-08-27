import assert from 'node:assert/strict';
import test from 'node:test';
import type { OpenCodeGoConnection } from '../types/openCodeGo.ts';
import {
  buildOpenCodeGoSummary,
  formatOpenCodeGoResetCountdown,
  pickOpenCodeGoSummaryConnection,
} from './openCodeGoSummary.ts';

const connection = (
  id: string,
  remaining: [number, number, number],
  queriedAt = 100,
): OpenCodeGoConnection => ({
  id,
  name: `Connection ${id}`,
  keyHint: 'ocg-****-key',
  createdAt: 1,
  updatedAt: queriedAt,
  quota: {
    rolling: { usedPercent: 100 - remaining[0], remainingPercent: remaining[0], resetsAt: 1000 },
    weekly: { usedPercent: 100 - remaining[1], remainingPercent: remaining[1], resetsAt: 2000 },
    monthly: { usedPercent: 100 - remaining[2], remainingPercent: remaining[2], resetsAt: 3000 },
    status: 'available',
    queriedAt,
  },
});

test('OpenCode Go summary uses only dedicated connection snapshots', () => {
  const summary = buildOpenCodeGoSummary([
    connection('go-primary', [80, 70, 60]),
  ]);

  assert.deepEqual(summary, {
    connectionCount: 1,
    healthyConnectionCount: 1,
    connection: {
      id: 'go-primary',
      name: 'Connection go-primary',
      keyHint: 'ocg-****-key',
      quota: {
        rolling: { usedPercent: 20, remainingPercent: 80, resetsAt: 1000 },
        weekly: { usedPercent: 30, remainingPercent: 70, resetsAt: 2000 },
        monthly: { usedPercent: 40, remainingPercent: 60, resetsAt: 3000 },
        status: 'available',
        queriedAt: 100,
      },
      quotaError: undefined,
    },
  });
});

test('summary connection selection prefers the safest available quota, then freshness', () => {
  const selected = pickOpenCodeGoSummaryConnection([
    connection('lower', [20, 90, 90], 300),
    connection('safer', [55, 60, 70], 100),
    connection('same-but-newer', [55, 60, 70], 400),
  ]);

  assert.equal(selected?.id, 'same-but-newer');
});

test('summary selection tolerates partial windows without fabricating numbers', () => {
  // Partial-window contract: a degraded window carries null numerics and
  // must be excluded from the safety score instead of coercing NaN into it.
  const partial: OpenCodeGoConnection = connection('partial', [40, 80, 60]);
  partial.quota = {
    ...partial.quota!,
    weekly: { status: 'error', usedPercent: null, remainingPercent: null, resetsAt: null, error: 'window unavailable' },
  };
  const selected = pickOpenCodeGoSummaryConnection([
    partial,
    connection('fully-known', [55, 90, 90], 400),
  ]);
  // partial's known minimum is 40 vs fully-known's 55 → fully-known wins.
  assert.equal(selected?.id, 'fully-known');
  const allUnknown: OpenCodeGoConnection = connection('blank', [0, 0, 0]);
  allUnknown.quota = {
    ...allUnknown.quota!,
    rolling: { status: 'unavailable', usedPercent: null, remainingPercent: null, resetsAt: null, error: 'window missing' },
    weekly: { status: 'error', usedPercent: null, remainingPercent: null, resetsAt: null, error: 'percent invalid' },
    monthly: { status: 'error', usedPercent: null, remainingPercent: null, resetsAt: null, error: 'reset missing' },
  };
  assert.equal(
    pickOpenCodeGoSummaryConnection([allUnknown])?.id,
    'blank',
  );
});

test('summary reports cached errors without manufacturing quota values', () => {
  const failed: OpenCodeGoConnection = {
    id: 'failed',
    name: 'Failed connection',
    keyHint: 'ocg-****-key',
    createdAt: 1,
    updatedAt: 2,
    quotaError: { kind: 'network', occurredAt: 3 },
  };

  assert.deepEqual(buildOpenCodeGoSummary([failed]), {
    connectionCount: 1,
    healthyConnectionCount: 0,
    connection: {
      id: 'failed',
      name: 'Failed connection',
      keyHint: 'ocg-****-key',
      quota: undefined,
      quotaError: { kind: 'network', occurredAt: 3 },
    },
  });
});

test('reset countdown is deterministic and accepts unix milliseconds defensively', () => {
  assert.equal(formatOpenCodeGoResetCountdown(4_660, 1_000), '1h 1m');
  assert.equal(formatOpenCodeGoResetCountdown(91_000_000_000, 90_910_000), '1d 1h');
  assert.equal(formatOpenCodeGoResetCountdown(900, 1_000), 'resetting');
  assert.equal(formatOpenCodeGoResetCountdown(undefined, 1_000), null);
  // Partial-window contract: a degraded window resetsAt is null, not undefined.
  assert.equal(formatOpenCodeGoResetCountdown(null, 1_000), null);
});
