import assert from 'node:assert/strict';
import test from 'node:test';
import { getMinimaxCodeQuotaSnapshot, getMinimaxCodePlanBadge, getMinimaxCodeUsage, minimaxCreditNumber, type MinimaxCodeAccount } from './minimaxCode';

const account: MinimaxCodeAccount = { id: 'fixture', email: 'fixture', access_token: '', created_at: 1, last_used: 1 };
test('official zero credit summary is known and an unsubscribed account has no invented plan badge', () => {
  const value = { ...account, quota_raw: { membership: { subscription_type: 'none', plan_name: '', op_credit_summary: {
    total_remaining_amount: '0', purchased_remaining_amount: '0', free_remaining_amount: '0',
  } } } };
  assert.equal(getMinimaxCodeQuotaSnapshot(value).total, 0);
  assert.equal(getMinimaxCodeQuotaSnapshot(value).unsubscribed, true);
  assert.equal(getMinimaxCodePlanBadge(value), '');
});
test('missing or invalid amounts never masquerade as zero', () => {
  for (const value of [undefined, null, '', ' ', 'invalid', NaN, Infinity, {}, false]) assert.equal(minimaxCreditNumber(value), null);
  assert.equal(minimaxCreditNumber('1,234.50'), 1234.5);
  assert.equal(getMinimaxCodeQuotaSnapshot(account).total, null);
  assert.equal(getMinimaxCodeUsage(account).isNormal, false);
});
test('cached balance survives a failed refresh while its status reports the error', () => {
  const value = { ...account, quota_query_last_error: 'HTTP 503', quota_raw: { membership: { opcredit_balance: 17, plan_name: 'M Plan Plus' } } };
  assert.equal(getMinimaxCodeQuotaSnapshot(value).total, 17);
  assert.equal(getMinimaxCodeUsage(value).isNormal, false);
  assert.equal(getMinimaxCodePlanBadge(value), 'M Plan Plus');
});
