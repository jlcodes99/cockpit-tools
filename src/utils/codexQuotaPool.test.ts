import assert from 'node:assert/strict';
import test from 'node:test';

import type { CodexAccount, CodexQuota } from '../types/codex.ts';
import { summarizeCodexQuotaPool } from './codexQuotaPool.ts';

function account(id: string, planType: string, quota?: CodexQuota): CodexAccount {
  return {
    id,
    auth_mode: 'oauth',
    plan_type: planType,
    quota,
  } as CodexAccount;
}

function quota(overrides: Partial<CodexQuota>): CodexQuota {
  return {
    hourly_percentage: 0,
    weekly_percentage: 0,
    ...overrides,
  };
}

test('treats a weekly primary window as weekly instead of 5h', () => {
  const summary = summarizeCodexQuotaPool([
    account(
      'plus-weekly-only',
      'plus',
      quota({
        hourly_percentage: 73,
        hourly_window_minutes: 10_080,
        hourly_window_present: true,
        weekly_window_present: false,
      }),
    ),
  ]);

  assert.deepEqual(summary.byPlan.PLUS.windows, [
    {
      key: 'weekly',
      label: 'Weekly',
      percentage: 73,
      accountCount: 1,
      windowMinutes: 10_080,
    },
  ]);
});

test('keeps legacy 5h and weekly windows when both are present', () => {
  const summary = summarizeCodexQuotaPool([
    account(
      'legacy-pro',
      'pro',
      quota({
        hourly_percentage: 80,
        weekly_percentage: 45,
        hourly_window_minutes: 300,
        weekly_window_minutes: 10_080,
        hourly_window_present: true,
        weekly_window_present: true,
      }),
    ),
  ]);

  assert.deepEqual(
    summary.byPlan.PRO.windows.map(({ label, percentage }) => ({ label, percentage })),
    [
      { label: '5h', percentage: 80 },
      { label: 'Weekly', percentage: 45 },
    ],
  );
});

test("shows an API key balance without adding it to OAuth quota windows", () => {
  const summary = summarizeCodexQuotaPool([
    {
      ...account(
        "legacy-api-key",
        "api_key",
        quota({
          hourly_percentage: 60,
          weekly_percentage: 25,
          raw_data: { usage: { total_available: 123.5 } },
        }),
      ),
      auth_mode: "apikey",
    } as CodexAccount,
  ]);

  assert.equal(summary.byPlan.API_KEY.count, 1);
  assert.equal(summary.byPlan.API_KEY.balance, 123.5);
  assert.deepEqual(summary.byPlan.API_KEY.windows, []);
});

test("uses the same cached provider balance shown on an API key account card", () => {
  const summary = summarizeCodexQuotaPool([
    {
      ...account(
        "custom-api-key",
        "api_key",
        quota({
          raw_data: {
            provider_usage: {
              mode: "sub2api",
              remaining: 5.6,
              unit: "USD",
            },
          },
        }),
      ),
      auth_mode: "apikey",
    } as CodexAccount,
  ]);

  assert.equal(summary.byPlan.API_KEY.balance, 5.6);
});

test('preserves a real zero-percent window and omits an account with no quota', () => {
  const summary = summarizeCodexQuotaPool([
    account(
      'plus-exhausted',
      'plus',
      quota({
        hourly_percentage: 0,
        hourly_window_minutes: 10_080,
        hourly_window_present: true,
        weekly_window_present: false,
      }),
    ),
    account('plus-missing', 'plus'),
  ]);

  assert.equal(summary.byPlan.PLUS.count, 2);
  assert.deepEqual(
    summary.byPlan.PLUS.windows.map(({ label, percentage, accountCount }) => ({
      label,
      percentage,
      accountCount,
    })),
    [{ label: 'Weekly', percentage: 0, accountCount: 1 }],
  );
});

test('merges equivalent weekly windows across mixed old and new accounts', () => {
  const summary = summarizeCodexQuotaPool([
    account(
      'plus-legacy',
      'plus',
      quota({
        hourly_percentage: 20,
        weekly_percentage: 40,
        hourly_window_minutes: 300,
        weekly_window_minutes: 10_080,
        hourly_window_present: true,
        weekly_window_present: true,
      }),
    ),
    account(
      'plus-weekly-only',
      'plus',
      quota({
        hourly_percentage: 30,
        hourly_window_minutes: 10_080,
        hourly_window_present: true,
        weekly_window_present: false,
      }),
    ),
  ]);

  assert.deepEqual(
    summary.byPlan.PLUS.windows.map(({ label, percentage, accountCount }) => ({
      label,
      percentage,
      accountCount,
    })),
    [
      { label: '5h', percentage: 20, accountCount: 1 },
      { label: 'Weekly', percentage: 70, accountCount: 2 },
    ],
  );
});

test("does not invent 5h windows for weekly-only plus or pro plans", () => {
  const accounts = ["plus", "pro"].map((planType, index) =>
    account(
      `${planType}-${index}`,
      planType,
      quota({
        hourly_percentage: 90 - index * 10,
        hourly_window_minutes: 10_080,
        hourly_window_present: true,
        weekly_window_present: false,
      }),
    ),
  );
  const summary = summarizeCodexQuotaPool(accounts);

  summary.visiblePlans.forEach((plan) => {
    assert.deepEqual(plan.windows.map((window) => window.label), ['Weekly']);
  });
});
