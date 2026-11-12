import assert from 'node:assert/strict';
import test from 'node:test';
import { buildCursorAccountPresentation } from '../presentation/platformAccountPresentation';
import { getCursorPlanBadge, getCursorPlanBadgeClass, getCursorPlanDisplayName, getCursorUsage, type CursorAccount } from './cursor';

const account = (extra: Partial<CursorAccount> = {}): CursorAccount => ({
  id: 'test', email: 'test@example.com', access_token: '', created_at: 0, last_used: 0, ...extra,
});

test('Cursor displays unified, legacy and future membership values verbatim', () => {
  for (const membership_type of ['Plus', 'Premium', 'Super', 'Ultra', 'Teams', 'pro_plus', 'pro_student', 'Future Tier']) {
    assert.equal(getCursorPlanDisplayName(account({ membership_type })), membership_type);
  }
  assert.equal(getCursorPlanDisplayName(account({ plan_type: 'Premium' })), 'Premium');
  assert.equal(getCursorPlanDisplayName(account({ membership_type: ' ', plan_type: 'Super' })), 'Super');
  assert.equal(getCursorPlanBadge(account({ plan_type: 'Premium' })), 'PREMIUM');
});

test('new tier styling preserves existing legacy filter categories', () => {
  assert.equal(getCursorPlanBadgeClass('Plus'), 'plus');
  assert.equal(getCursorPlanBadgeClass('Premium'), 'pro');
  assert.equal(getCursorPlanBadgeClass('Super'), 'ultra');
  assert.equal(getCursorPlanBadge(account({ membership_type: 'pro_student' })), 'PRO');
});

test('legacy Cursor usage keeps plan usage separate from paid additional usage', () => {
  const usage = getCursorUsage(account({ membership_type: 'Premium', cursor_usage_raw: {
    billingCycleEnd: '2026-11-12T00:00:00Z',
    individualUsage: { plan: { used: 500, limit: 2000, totalPercentUsed: 25 }, onDemand: { used: 800, limit: 1000, enabled: true } },
  } }));
  assert.equal(usage.inlineSuggestionsUsedPercent, 25);
  assert.equal(usage.onDemandUsedCents, 800);
  assert.equal(usage.allowanceResetAt, Date.parse('2026-11-12T00:00:00Z') / 1000);
});

test('Cursor uses the official weekly reset without falling back to the billing cycle', () => {
  const base = { billingCycleEnd: '2026-12-01T00:00:00Z' };
  const includedUsageResetsAt = Date.parse('2026-11-12T00:00:00Z');
  const membership_type = 'unified_ultra';
  const usage = (cursor_usage_raw: unknown) => getCursorUsage(account({ membership_type, cursor_usage_raw }));

  assert.equal(usage(base).allowanceResetAt, null);
  assert.equal(usage({
    ...base,
    planInfo: { includedUsagePeriod: 'INCLUDED_USAGE_PERIOD_WEEKLY', includedUsageResetsAt: String(includedUsageResetsAt) },
  }).allowanceResetAt, includedUsageResetsAt / 1000);
  assert.equal(usage({ ...base, included_usage_resets_at: includedUsageResetsAt }).allowanceResetAt, includedUsageResetsAt / 1000);
});

test('weekly Cursor presentation preserves true zeros, unset caps and the auto top-up rule', () => {
  const items = cursorQuotaItems({
    planInfo: { includedUsagePeriod: 2, includedUsageResetsAt: '1794755940000' },
    planUsage: { totalPercentUsed: 0, autoPercentUsed: 30, apiPercentUsed: 50, limit: 4615 },
    spendLimitUsage: { individualUsed: 1500 },
    credits: {
      balanceCents: 0, monthlyUsedCents: 0, monthlyLimitCents: null,
      autoTopUpEnabled: true, autoTopUpThresholdCents: 500, autoTopUpAmountCents: 2000,
    },
  });
  assert.deepEqual(items.map(({ key, valueText }) => [key, valueText]), [
    ['total', 'cursor.quota.usedPercent:0'],
    ['balance', '$0.00'],
    ['monthly_used', '$0.00'],
    ['monthly_limit', 'cursor.quota.limitUnset'],
    ['auto_top_up', 'common.enabled'],
  ]);
  assert.equal(items[0].resetAt, 1794755940);
  assert.equal(items[items.length - 1].resetText, 'cursor.quota.autoTopUpDetails:$5.00,$20.00');
  assert.ok(items.slice(1).every(({ showProgress }) => showProgress === false));
});

test('weekly Cursor presentation never substitutes the old web total or reset for missing values', () => {
  const items = cursorQuotaItems({
    planInfo: { includedUsagePeriod: 'INCLUDED_USAGE_PERIOD_WEEKLY' },
    billingCycleEnd: '2026-12-01T00:00:00Z',
    individualUsage: { plan: { totalPercentUsed: 75 }, onDemand: { enabled: false } },
    credits: { autoTopUpEnabled: false, monthlyLimitCents: 0 },
  });
  assert.equal(items[0].valueText, '—');
  assert.equal(items[0].showProgress, false);
  assert.equal(items[0].resetText, 'common.shared.quota.resetAt:—');
  assert.equal(items[1].valueText, '—');
  assert.equal(items[2].valueText, '—');
  assert.equal(items[3].valueText, '$0.00');
  assert.equal(items[4].valueText, 'common.disabled');
});

test('legacy Cursor presents only reported allowance and paid usage details', () => {
  const items = cursorQuotaItems({
    planInfo: { includedUsagePeriod: 1 },
    planUsage: { autoPercentUsed: 0, apiPercentUsed: 37 },
    spendLimitUsage: { individualLimit: 1000 },
  });
  assert.deepEqual(items.map(({ key, valueText }) => [key, valueText]), [
    ['auto', 'cursor.quota.usedPercent:0'],
    ['api', 'cursor.quota.usedPercent:37'],
    ['on_demand', '—'],
    ['sand', 'cursor.quota.usedPercent:13'],
  ]);
  assert.equal(items[2].resetText, '— / $10.00');
});

function cursorQuotaItems(cursor_usage_raw: unknown) {
  const t = (key: string, options?: unknown) =>
    options && typeof options === 'object' ? `${key}:${Object.values(options).join(',')}` : key;
  return buildCursorAccountPresentation(account({
    membership_type: 'premium',
    cursor_usage_raw,
    sand_usage: { percent_used: 13, reset_at: null, stale: false },
  }), t).quotaItems;
}
