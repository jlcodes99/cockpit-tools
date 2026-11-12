import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import { summarizeCodexQuotaPool } from './codexQuotaPool';
import { summarizeCodexProviderPoolQuotas } from './codexProviderPoolQuota';

function account(id: string, raw_data?: unknown): CodexAccount {
  return {
    id,
    auth_mode: 'apikey',
    plan_type: 'api_key',
    api_base_url: 'https://relay.example.invalid/v1',
    api_provider_name: id,
    usage_updated_at: 1000,
    quota: { hourly_percentage: 94, weekly_percentage: 0, raw_data },
  } as CodexAccount;
}

test('provider balances are separate from subscription windows and keep source units', () => {
  const percentage = account('Percent relay', {
    object: 'codex_cockpit_api_quota',
    usage: { total_available: 940_000, total_granted: 1_000_000, display: { unit: '%', remaining: 94, total: 100 } },
  });
  const wallet = account('Wallet relay', {
    provider_usage: { mode: 'new_api', balance: 32, unit: 'CNY', quotaUnlimited: true },
  });
  const subscription = {
    id: 'plus',
    auth_mode: 'oauth',
    plan_type: 'plus',
    quota: { hourly_percentage: 60, weekly_percentage: 20 },
  } as CodexAccount;
  const pool = summarizeCodexQuotaPool([percentage, wallet, subscription]);
  assert.deepEqual(pool.all.windows.map((window) => window.percentage), [60, 20]);
  assert.equal(pool.byPlan.API_KEY.count, 2);
  assert.deepEqual(pool.byPlan.API_KEY.windows, []);
  assert.deepEqual(
    summarizeCodexProviderPoolQuotas([percentage, wallet, subscription], 1001)
      .map(({ name, remaining, total, unit, status }) => [name, remaining, total, unit, status]),
    [['Percent relay', 94, 100, '%', 'ready'], ['Wallet relay', 32, null, 'CNY', 'ready']],
  );
});

test('reports missing, failed and stale data without guessing units or balances', () => {
  const missing = account('No query');
  const failed = account('Failed', { provider_usage: { isValid: false } });
  const stale = { ...account('Stale', { provider_usage: { quotaRemaining: 0, unit: 'USD' } }), usage_updated_at: 90 };
  const legacy = account('Legacy', {
    object: 'codex_cockpit_api_quota',
    total_available: 0, // Legacy synthesized values do not establish a real zero.
    usage: { total_granted: 100 },
  });
  const rawUnit = account('Unknown unit', { object: 'codex_cockpit_api_quota', usage: { total_available: 44, total_granted: 100 } });
  const unlimited = account('Unlimited without wallet', { object: 'codex_cockpit_api_quota', usage: { total_available: 0, unlimited_quota: true } });
  const rows = summarizeCodexProviderPoolQuotas([missing, failed, stale, legacy, rawUnit, unlimited], 1001);
  assert.deepEqual(rows.map(({ status, remaining, unit, unlimited }) => [status, remaining, unit, unlimited]), [
    ['unavailable', null, null, false],
    ['failed', null, null, false],
    ['stale', 0, 'USD', false],
    ['unavailable', null, null, false],
    ['ready', 44, null, false],
    ['ready', null, null, true],
  ]);
});
