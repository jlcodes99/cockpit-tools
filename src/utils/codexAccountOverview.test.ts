import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount, CodexQuota } from '../types/codex.ts';
import { createCodexOverviewAccountComparator } from './codexAccountOverview.ts';

const account = (id: string, quota?: Partial<CodexQuota>) => ({
  id, created_at: 1, auth_mode: 'oauth', quota,
} as CodexAccount);
const compare = (sortBy: string, sortDirection: 'asc' | 'desc' = 'asc') =>
  createCodexOverviewAccountComparator({ sortBy, sortDirection,
    customSortOrder: [], currentAccountId: null, resolveSubscriptionTimestamp: () => null });

test('weekly sorting recognizes the weekly primary window used by free accounts', () => {
  const free = account('free', { hourly_percentage: 70, hourly_reset_time: 200,
    hourly_window_minutes: 10080, hourly_window_present: true, weekly_window_present: false });
  const plus = account('plus', { hourly_percentage: 100, weekly_percentage: 30,
    weekly_reset_time: 300 });
  assert.ok(compare('weekly')(free, plus) > 0);
  assert.ok(compare('weekly', 'desc')(free, plus) < 0);
  assert.ok(compare('weekly_reset')(free, plus) < 0);
  assert.ok(compare('weekly_reset', 'desc')(free, plus) > 0);
  assert.ok(compare('hourly')(free, plus) > 0);
});

test('missing windows stay after known zero quota in either direction', () => {
  const missing = account('missing');
  const zero = account('zero', { hourly_percentage: 0, weekly_percentage: 0 });
  for (const direction of ['asc', 'desc'] as const) {
    assert.ok(compare('weekly', direction)(missing, zero) > 0);
    assert.ok(compare('hourly', direction)(missing, zero) > 0);
  }
});

test('weekly exhaustion sorts using the same effective hourly quota displayed in cards', () => {
  const exhausted = account('exhausted', { hourly_percentage: 100, weekly_percentage: 0 });
  const available = account('available', { hourly_percentage: 50, weekly_percentage: 50 });
  assert.ok(compare('hourly')(exhausted, available) < 0);
});
