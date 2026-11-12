import assert from 'node:assert/strict';
import test from 'node:test';
import { getGrokPlanBadge, getGrokQuotaSummaryItems, getGrokUsage, type GrokAccount } from './grok';

const account = (extra: Partial<GrokAccount> = {}): GrokAccount => ({
  id: 'test', email: 'test@example.com', access_token: '', created_at: 0, last_used: 0, ...extra,
});
const t = (key: string, fallback?: string) => fallback || key;

test('Grok preserves unified, legacy and future subscription values verbatim', () => {
  for (const plan_type of ['Plus', 'Premium', 'Super', 'Ultra', 'Teams', 'Enterprise', 'SUBSCRIPTION_TIER_SUPERGROK', 'Future Plan']) {
    assert.equal(getGrokPlanBadge(account({ plan_type })), plan_type);
  }
  assert.equal(getGrokPlanBadge(account({ quota: { subscriptionTier: 'Premium', products: [] } })), 'Premium');
  assert.equal(getGrokPlanBadge(account()), '');
});

test('unified allowance is one pool; product shares and old spending caps do not exhaust it', () => {
  const unified = account({ quota: {
    isUnifiedBillingUser: true, weeklyLimitPercent: 20,
    periodType: 'USAGE_PERIOD_TYPE_WEEKLY', periodEnd: '2026-10-19T00:00:00Z',
    products: [{ product: 'Cursor', usagePercent: 100 }],
    frequentUsage: 10, frequentLimit: 10, onDemandUsed: 100, onDemandCap: 100,
  } });
  const rows = getGrokQuotaSummaryItems(unified, t);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].label, '共享额度');
  assert.equal(rows[0].percentage, 20);
  assert.equal(rows[0].resetAtMs, Date.parse('2026-10-19T00:00:00Z'));
  assert.equal(getGrokUsage(unified).exhausted, false);
  assert.equal(getGrokUsage(unified).totalUsedPercent, 20);
});

test('legacy monthly and product/task allowances remain available; disabled on-demand is excluded', () => {
  const rows = getGrokQuotaSummaryItems(account({ quota: {
    periodType: 'USAGE_PERIOD_TYPE_MONTHLY', weeklyLimitPercent: 25,
    products: [{ product: 'GrokBuild', usagePercent: 60 }], frequentUsage: 2, frequentLimit: 10,
    onDemandEnabled: false, onDemandUsed: 100, onDemandCap: 100,
  } }), t);
  assert.deepEqual(rows.map(row => row.key), ['weekly', 'product-0', 'frequent']);
  assert.equal(rows[0].label, '每月用量');
});

test('legacy Heavy accounts display the reported tier, weekly quota and GrokBuild quota', () => {
  const heavy = account({ quota: {
    subscriptionTier: 'SUBSCRIPTION_TIER_SUPER_GROK_HEAVY',
    periodType: 'weekly',
    periodEnd: '2026-10-19T00:00:00Z',
    weeklyLimitPercent: 18,
    products: [{ product: 'GrokBuild', usagePercent: 32, used: 32, total: 100, remaining: 68 }],
  } });

  assert.equal(getGrokPlanBadge(heavy), 'SUBSCRIPTION_TIER_SUPER_GROK_HEAVY');
  assert.deepEqual(getGrokQuotaSummaryItems(heavy, t).map(
    ({ label, percentage, resetAtMs }) => [label, percentage, resetAtMs],
  ), [
    ['每周用量', 18, Date.parse('2026-10-19T00:00:00Z')],
    ['GrokBuild', 32, Date.parse('2026-10-19T00:00:00Z')],
  ]);
});

test('missing shared usage stays unknown and API keys do not inherit subscription pools', () => {
  assert.deepEqual(getGrokQuotaSummaryItems(account({ quota: { isUnifiedBillingUser: true, products: [] } }), t), []);
  assert.deepEqual(getGrokQuotaSummaryItems(account({ auth_mode: 'api_key', quota: { weeklyLimitPercent: 20, products: [] } }), t), []);
});

test('zero weekly usage is a real quota and legacy task slots are separate', () => {
  const periodEnd = '2026-10-19T00:00:00Z';
  const tasks = { frequentUsage: 0, frequentLimit: 2, occasionalUsage: 0, occasionalLimit: 10, products: [] };
  const zero = account({ quota: {
    ...tasks, isUnifiedBillingUser: true, weeklyLimitPercent: 0, periodType: 'weekly', periodEnd,
  } });
  const taskOnly = account({ quota: tasks });
  assert.deepEqual(getGrokQuotaSummaryItems(zero, t).map(
    ({ key, percentage, resetAtMs }) => [key, percentage, resetAtMs],
  ), [['weekly', 0, Date.parse(periodEnd)]]);
  assert.deepEqual(getGrokQuotaSummaryItems(taskOnly, t).map(
    ({ key, resetAtMs }) => [key, resetAtMs],
  ), [['frequent', null], ['occasional', null]]);
});
