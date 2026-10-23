import assert from 'node:assert/strict';
import test from 'node:test';

import type { CodexAccount, CodexQuota } from '../types/codex.ts';
import { formatCodexQuotaPoolPlanLabel, summarizeCodexQuotaPool } from './codexQuotaPool.ts';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';
import { projectCodexServiceAccountDisplays, reconcileChatGptPlanDisplaySources } from './chatgptPlanOverview';

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

test('uses legacy labels when window metadata is unavailable', () => {
  const summary = summarizeCodexQuotaPool([
    account(
      'legacy-api-key',
      'api_key',
      quota({ hourly_percentage: 60, weekly_percentage: 25 }),
    ),
  ]);

  assert.deepEqual(
    summary.byPlan.API_KEY.windows.map(({ label, percentage }) => ({ label, percentage })),
    [
      { label: '5h', percentage: 60 },
      { label: 'Weekly', percentage: 25 },
    ],
  );
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

test('does not invent 5h windows for weekly-only plus, pro, or API key plans', () => {
  const accounts = ['plus', 'pro', 'api_key'].map((planType, index) =>
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

const officialAccount = (id: string, planType = '') => ({
  ...account(`chatgpt_plan_${id}`, planType), auth_mode: 'apikey', api_provider_id: 'cockpit_chatgpt_plan',
}) as CodexAccount;
const boundGrant = (id: string, sourceId: string) => ({ id, source_account_id: sourceId, source_binding_valid: true }) as ChatGptPlanAccount;

test('groups bound official members under the source subscription with source quota', () => {
  const local = account('local', 'Plus', quota({ hourly_percentage: 60, hourly_window_present: true, weekly_window_present: false }));
  const source = account('source', 'Plus', quota({ hourly_percentage: 99, hourly_window_present: true, weekly_window_present: false }));
  const official = { ...officialAccount('grant', 'stale plan'), quota: source.quota };
  const summary = summarizeCodexQuotaPool([local, official], {
    sourceAccounts: [local, source, official], grants: [boundGrant('grant', 'source')],
  });
  assert.equal(summary.all.count, 2, 'the source metadata is not an extra member');
  assert.deepEqual(summary.visiblePlans.map((item) => [formatCodexQuotaPoolPlanLabel(item, '官'), item.count]), [
    ['PLUS', 2],
  ]);
  assert.equal(summary.byPlan.PLUS.windows[0].percentage, 159);
  assert.equal(summary.byPlan['CHATGPT_PLAN:PLUS'], undefined);
  assert.equal(summary.all.windows[0].percentage, 159, 'observed source quota is counted once per source');
  assert.equal(official.plan_type, 'stale plan', 'display aggregation does not mutate the endpoint');
});

test('subscription statistics do not split a plan by authorization channel', () => {
  const summary = summarizeCodexQuotaPool([
    officialAccount('pro', 'Pro 100'), account('pro', 'Pro 100'),
    officialAccount('plus', 'Plus'), account('plus', 'Plus'), officialAccount('business', 'Business Special Edition'),
  ]);
  assert.deepEqual(summary.visiblePlans.map((item) => formatCodexQuotaPoolPlanLabel(item, 'Official')), [
    'PLUS', 'PRO', 'BUSINESS SPECIAL EDITION',
  ]);
});

test('invalid, ambiguous or missing source bindings never borrow another account subscription', () => {
  const source = account('source', 'Plus');
  const endpoint = officialAccount('grant');
  for (const grants of [
    [], [boundGrant('other', source.id)],
    [{ ...boundGrant('grant', source.id), source_binding_valid: false }],
    [boundGrant('grant', source.id), boundGrant('duplicate', source.id)],
    [boundGrant('grant', 'missing')],
  ]) {
    const summary = summarizeCodexQuotaPool([endpoint], { sourceAccounts: [source], grants });
    assert.equal(summary.visiblePlans.length, 1);
    assert.equal(formatCodexQuotaPoolPlanLabel(summary.visiblePlans[0], '官'), 'ChatGPT');
    assert.equal(summary.byPlan.PLUS, undefined);
  }
});

test('existing dual membership counts the original subscription once', () => {
  const source = account('source', 'TEAM');
  const endpoint = officialAccount('grant');
  const summary = summarizeCodexQuotaPool([source, endpoint], {
    sourceAccounts: [source, endpoint], grants: [boundGrant('grant', source.id)],
  });
  assert.deepEqual(summary.visiblePlans.map((item) => [item.label, item.official, item.count]), [
    ['TEAM', false, 1],
  ]);
});

test('TEAM and PLUS statistics keep their existing uppercase labels across pending rechecks', () => {
  const source = account('source', 'plus');
  const endpoint = officialAccount('grant');
  const team = account('team', 'team');
  const linked = boundGrant('grant', source.id);
  const previous = reconcileChatGptPlanDisplaySources([linked]);
  const pending = { ...linked, source_binding_valid: false, source_binding_error: null };
  const state = reconcileChatGptPlanDisplaySources([pending], previous.confirmed);
  const context = { sourceAccounts: [source, endpoint, team], grants: [pending], displaySourceIds: state.displaySourceIds };
  const labels = { pending: '关联校验中', invalid: '账号关联异常' };
  const summary = summarizeCodexQuotaPool([team, endpoint], context);
  assert.deepEqual(summary.visiblePlans.map((item) => formatCodexQuotaPoolPlanLabel(item, '官', labels)), ['PLUS', 'TEAM']);
  assert.equal(pending.source_binding_valid, false);
  assert.equal(summary.all.count, 2);
  const cold = summarizeCodexQuotaPool([endpoint], { ...context, displaySourceIds: new Map() });
  assert.equal(formatCodexQuotaPoolPlanLabel(cold.visiblePlans[0], '官', labels), '关联校验中');
  const invalid = summarizeCodexQuotaPool([endpoint], { ...context, grants: [{ ...pending, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }] });
  assert.equal(formatCodexQuotaPoolPlanLabel(invalid.visiblePlans[0], '官', labels), '账号关联异常');
  assert.deepEqual(invalid.all.windows, []);
});

test('an invalidated display projection cannot contribute stale source quota or subscription', () => {
  const source = account('source', 'Plus', quota({ hourly_percentage: 88, weekly_percentage: 0 }));
  const endpoint = officialAccount('grant');
  const grant = boundGrant('grant', source.id);
  const projected = projectCodexServiceAccountDisplays([source, endpoint], [grant])[1];
  const valid = summarizeCodexQuotaPool([projected]);
  assert.equal(valid.byPlan.PLUS.windows[0].percentage, 0, 'a zero weekly allowance still blocks the short window');
  const invalid = summarizeCodexQuotaPool([projected], {
    sourceAccounts: [source], grants: [{ ...grant, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }],
  });
  assert.equal(invalid.byPlan.PLUS, undefined);
  assert.equal(invalid.all.count, 1);
  assert.deepEqual(invalid.all.windows, []);
});
