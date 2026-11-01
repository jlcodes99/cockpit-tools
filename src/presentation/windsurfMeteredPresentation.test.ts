import assert from 'node:assert/strict';
import test from 'node:test';
import { buildWindsurfAccountPresentation } from './platformAccountPresentation.ts';
import { getWindsurfOfficialUsageMode, getWindsurfMeteredUsage, type WindsurfAccount } from '../types/windsurf.ts';

const base = { id: 'fixture', github_login: 'fixture', github_id: 1, github_access_token: '', copilot_token: '', created_at: 0, last_used: 0 } satisfies WindsurfAccount;
const t = (key: string, options?: unknown) => (options && typeof options === 'object' && 'unit' in options) ? String(options.unit) : key;

test('ACU and dollar quotas retain fractional units and correct billing mode', () => {
  const account = { ...base, windsurf_plan_status: { planInfo: { billingStrategy: 'BILLING_STRATEGY_ACU' }, acuConsumed: 2.5, acuLimit: 10, dollarsConsumed: 1.25, dollarsLimit: 5 } };
  assert.equal(getWindsurfOfficialUsageMode(account), 'acu');
  const items = buildWindsurfAccountPresentation(account, t).quotaItems;
  assert.deepEqual(items.map(item => [item.key, item.percentage, item.label]), [['acu', 25, 'ACU'], ['dollars', 25, 'USD']]);
  assert.equal(items[0].valueText, '2.5 / 10');
});

test('missing or zero metered limits never become fabricated progress', () => {
  const account = { ...base, windsurf_plan_status: { planInfo: { billingStrategy: 3 }, acuConsumed: 0, dollarsConsumed: 0, dollarsLimit: 0 } };
  assert.equal(getWindsurfOfficialUsageMode(account), 'acu');
  const items = buildWindsurfAccountPresentation(account, t).quotaItems;
  assert.ok(items.every(item => item.showProgress === false));
  assert.equal(items[0].valueText, '0 / --');
  assert.equal(items[1].valueText, '0 / 0');
  assert.equal(getWindsurfMeteredUsage(base).length, 0);
});

test('official hide flags apply to daily and weekly quota presentation', () => {
  const account = { ...base, windsurf_plan_status: { planInfo: { billingStrategy: 'BILLING_STRATEGY_QUOTA', hideDailyQuota: true, hideWeeklyQuota: true } } };
  const items = buildWindsurfAccountPresentation(account, t).quotaItems;
  assert.ok(items.every(item => !['daily_quota', 'weekly_quota'].includes(item.key)));
});
