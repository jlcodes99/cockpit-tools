import assert from 'node:assert/strict';
import test from 'node:test';
import { getTraeUsage, type TraeAccount } from './trae.ts';
import { buildTraeAccountPresentation } from '../presentation/platformAccountPresentation.ts';
const base: TraeAccount = { id: 'fixture', email: 'fixture@example.test', access_token: 'fixture', created_at: 0, last_used: 0 };
const pack = (limit: number, used: number) => ({ entitlement_base_info: { product_type: 0, quota: { credits_limit: limit } }, usage: { credits_amount: used } });
const account = (...packs: unknown[]) => ({ ...base, trae_usage_raw: { code: 0, data: { is_credits_billing: true, user_entitlement_pack_list: packs } } });
test('TRAE credit billing aggregates fractional package balances without USD', () => {
  const a = account(pack(100, 25), pack(50, 12.5));
  const usage = getTraeUsage(a);
  assert.equal(usage.usageModel, 'credits');
  assert.equal(usage.creditsUsed, 37.5);
  assert.equal(usage.creditsLimit, 150);
  assert.equal(usage.creditsRemaining, 112.5);
  assert.equal(usage.usedPercent, 25);
  assert.equal(usage.spentUsd, null);
  assert.equal(buildTraeAccountPresentation(a, key => key).quotaItems[0].valueText, '37.5 / 150');
});
test('unlimited and missing TRAE credit limits do not become zero-dollar quotas', () => {
  const unlimited = getTraeUsage(account(pack(-1, 5)));
  assert.equal(unlimited.creditsUnlimited, true);
  assert.equal(unlimited.usedPercent, null);
  assert.equal(unlimited.usageExhausted, false);
  const unknown = getTraeUsage(account({}));
  assert.equal(unknown.creditsUsed, null);
  assert.equal(unknown.creditsLimit, null);
  assert.equal(unknown.usedPercent, null);
});
