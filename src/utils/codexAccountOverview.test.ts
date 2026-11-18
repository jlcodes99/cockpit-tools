import assert from 'node:assert/strict';
import test from 'node:test';
import type { TFunction } from 'i18next';
import type { CodexAccount, CodexQuota } from '../types/codex.ts';
import { getCodexSubscriptionExpiryBucket } from '../types/codex.ts';
import { buildCodexOverviewSortOptions, createCodexOverviewAccountComparator, isCodexOverviewAccountSubscriptionExpired } from './codexAccountOverview.ts';

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

test('credits sort uses remaining amounts, including zero and unlimited balances', () => {
  const unknown = account('unknown', { raw_data: { spend_control: { individual_limit: { limit: 100 } } } });
  const zero = account('zero', { raw_data: { credits: { balance: '0' } } });
  const computed = account('computed', { raw_data: { spend_control: { individual_limit: { limit: 100, used: 97 } } } });
  const paid = account('paid', { raw_data: { credits: { balance: '12.5' } } });
  const unlimited = account('unlimited', { raw_data: { credits: { unlimited: true } } });
  const apiKey = { ...paid, id: 'api-key', auth_mode: 'apikey' as const };
  const accounts = [unknown, unlimited, paid, zero, computed, apiKey];
  const sort = (sortDirection: 'asc' | 'desc') => createCodexOverviewAccountComparator({
    sortBy: 'credits', sortDirection, currentAccountId: 'computed', customSortOrder: [],
    resolveSubscriptionTimestamp: () => null,
  });
  assert.deepEqual([...accounts].sort(sort('asc')).map(({ id }) => id),
    ['zero', 'computed', 'paid', 'unlimited', 'unknown', 'api-key']);
  assert.deepEqual([...accounts].sort(sort('desc')).map(({ id }) => id),
    ['unlimited', 'paid', 'computed', 'zero', 'unknown', 'api-key']);
  assert.equal(sort('desc')(unlimited, { ...unlimited, id: 'second-unlimited' }), 0);
  const t = ((key: string) => key) as TFunction;
  assert.ok(buildCodexOverviewSortOptions(t).some(({ value }) => value === 'credits'));
});

test('the expired filter follows the card for numeric and ISO subscription timestamps', () => {
  for (const timestamp of [Date.UTC(2022, 0, 1), Date.UTC(2090, 0, 1)]) {
    for (const expiry of [String(timestamp / 1000), String(timestamp), new Date(timestamp).toISOString()]) {
      const source = { ...account('fixture'), subscription_active_until: expiry };
      const expired = timestamp < Date.now();
      assert.equal(getCodexSubscriptionExpiryBucket(expiry) === 'expired', expired);
      assert.equal(isCodexOverviewAccountSubscriptionExpired(source), expired);
    }
  }
  assert.equal(isCodexOverviewAccountSubscriptionExpired({ ...account('fixture'), subscription_active_until: 'unknown' }), false);
});
