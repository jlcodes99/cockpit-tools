import assert from 'node:assert/strict';
import test from 'node:test';
import { getQoderSubscriptionInfo, resolveQoderCreditUsage, type QoderAccount } from './qoder.ts';
const base: QoderAccount = { id: 'fixture', email: 'fixture@example.test', created_at: 0, last_used: 0 };

test('hybrid usage reads only the official personal quota bucket', () => {
  const account = { ...base, auth_credit_usage_raw: { displayMode: 'hybrid', qoderUsage: { userQuota: { used: 1, total: 100, remaining: 99, percentage: 1 } }, enterpriseUsage: { used: 999 } } };
  const info = getQoderSubscriptionInfo(account);
  assert.equal(info.userQuota.used, 1);
  assert.equal(info.userQuota.percentage, 1);
});

test('enterprise-only usage cannot expose cached personal credits', () => {
  const account = { ...base, credits_used: 50, credits_total: 100, credits_remaining: 50, credits_usage_percent: 50,
    auth_user_plan_raw: { userQuota: { used: 75, total: 100 } },
    auth_credit_usage_raw: { displayMode: 'enterprise', qoderUsage: { used: 99 }, enterpriseUsage: { detailUrl: 'https://qoder.com/usage' } } };
  const info = getQoderSubscriptionInfo(account);
  assert.equal(resolveQoderCreditUsage(account), null);
  assert.deepEqual(info.userQuota, { used: null, total: null, remaining: null, percentage: null });
  assert.equal(info.totalUsagePercentage, null);
});

test('old flat quota snapshots remain readable', () => {
  const raw = { userQuota: { used: 2, total: 10, remaining: 8, percentage: 20 } };
  assert.equal(resolveQoderCreditUsage({ ...base, auth_credit_usage_raw: raw }), raw);
  assert.equal(getQoderSubscriptionInfo({ ...base, auth_credit_usage_raw: raw }).userQuota.percentage, 20);
});
