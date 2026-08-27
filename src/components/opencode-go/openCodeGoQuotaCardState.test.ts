import assert from 'node:assert/strict';
import test from 'node:test';
import {
  buildOpenCodeGoQuotaCardStates,
  formatOpenCodeGoResetCountdown,
} from './openCodeGoQuotaCardState.ts';

test('rolling, weekly, and monthly windows keep independent partial states', () => {
  const cards = buildOpenCodeGoQuotaCardStates({
    windows: {
      rolling: {
        usedPercent: 25.5,
        remainingPercent: 74.5,
        resetsAt: 1_700_007_500,
      },
      monthly: {
        usedPercent: null,
        remainingPercent: 40,
        resetsAt: null,
      },
    },
    loadingWindows: ['weekly'],
    nowMs: 1_700_000_000_000,
  });

  assert.deepEqual(
    cards.map(({ id, label, status, percentageText, resetText }) => ({
      id,
      label,
      status,
      percentageText,
      resetText,
    })),
    [
      {
        id: 'rolling',
        label: 'Rolling 5h',
        status: 'ready',
        percentageText: '74.5%',
        resetText: 'Resets in 2h 5m',
      },
      {
        id: 'weekly',
        label: 'Weekly',
        status: 'loading',
        percentageText: 'Loading…',
        resetText: '',
      },
      {
        id: 'monthly',
        label: 'Monthly',
        status: 'ready',
        percentageText: '40%',
        resetText: 'Reset unavailable',
      },
    ],
  );
});

test('a failed window does not hide available or unavailable siblings', () => {
  const cards = buildOpenCodeGoQuotaCardStates({
    windows: {
      rolling: { remainingPercent: 110, resetsAt: 1_699_999_999 },
    },
    errors: { weekly: 'Weekly quota unavailable' },
    nowMs: 1_700_000_000_000,
  });

  assert.equal(cards[0].status, 'ready');
  assert.equal(cards[0].remainingPercent, 100);
  assert.equal(cards[0].resetText, 'Reset due');
  assert.deepEqual(
    cards.slice(1).map(({ status, percentageText }) => ({ status, percentageText })),
    [
      { status: 'error', percentageText: 'Weekly quota unavailable' },
      { status: 'unavailable', percentageText: 'Unavailable' },
    ],
  );
});

test('reset countdown is deterministic at day, minute, and expired boundaries', () => {
  const nowMs = 1_700_000_000_000;
  assert.equal(
    formatOpenCodeGoResetCountdown(1_700_093_840, nowMs),
    'Resets in 1d 2h',
  );
  assert.equal(
    formatOpenCodeGoResetCountdown(1_700_000_059, nowMs),
    'Resets in <1m',
  );
  assert.equal(formatOpenCodeGoResetCountdown(1_700_000_000, nowMs), 'Reset due');
  assert.equal(formatOpenCodeGoResetCountdown(null, nowMs), 'Reset unavailable');
});
