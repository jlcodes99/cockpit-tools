import test from 'node:test';
import assert from 'node:assert/strict';
import { budgetDraft, budgetPeriodStart, budgetCurrentUsed, parseBudgetDraft, parseBudgetValue } from './codexKeyBudget';

test('budget parsing retains microdollar precision and rejects zero, exponent and unsafe values', () => {
  assert.equal(parseBudgetValue('0.000001', true), 1);
  for (const value of ['0', '-1', '1e6', 'Infinity', '0.0000001']) assert.equal(parseBudgetValue(value, true), undefined);
  assert.equal(parseBudgetValue('', true), null);
  assert.equal(parseBudgetValue('1025', false, 1024), undefined);
  const draft = { ...budgetDraft(), period: 'monthly' as const, amount: '20', dailyAmount: '3', maxConcurrent: '2' };
  assert.deepEqual(parseBudgetDraft(draft), { period: 'monthly', limitMicrousd: 20_000_000, dailyLimitMicrousd: 3_000_000, maxConcurrent: 2, dailyImageLimit: null });
});

test('budget display follows UTC periods independently of the local timezone', () => {
  const now = Date.parse('2026-11-01T00:01:00Z');
  assert.equal(budgetPeriodStart('monthly', now), Date.parse('2026-11-01T00:00:00Z'));
  assert.equal(budgetPeriodStart('weekly', now), Date.parse('2026-10-26T00:00:00Z'));
  const policy = parseBudgetDraft({ ...budgetDraft(), period: 'monthly', amount: '20' })!;
  const bucket = { start: Date.parse('2026-10-01T00:00:00Z'), usedMicrousd: 4_000_000, imageRequests: 0 };
  assert.equal(budgetCurrentUsed(policy, { lifetimeMicrousd: 4_000_000, unmeteredRequests: 0, daily: bucket, weekly: bucket, monthly: bucket }, now), 0);
});
