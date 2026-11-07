import assert from 'node:assert/strict';
import test from 'node:test';
import { buildCodexAccountPresentation } from './platformAccountPresentation';
import { getCodexPlanBadgePresentation, getCodexPlanFilterKey, isCodexEffectiveFreePlan, type CodexAccount } from '../types/codex';
import { summarizeCodexQuotaPool } from '../utils/codexQuotaPool';
import { buildCodexPlanFilterOptions, createCodexPlanFilterCounts, incrementCodexPlanFilterCount, filterAndSortCodexOverviewAccounts } from '../utils/codexAccountOverview';
import { projectCodexServiceAccountDisplays } from '../utils/chatgptPlanOverview';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';
import { isCodexLocalAccessEligibleAccount } from '../utils/codexLocalAccessAccounts';

const plan = { id: 'chatgpt_plan_registration', account_name: 'Plan account', email: 'user@example.com', auth_mode: 'apikey', api_provider_id: 'cockpit_chatgpt_plan' } as CodexAccount;
const t = (key: string) => key === 'chatgptPlan.title' ? 'ChatGPT 套餐' : key;

test('every common account presentation identifies the plan source without inventing a Free subscription or quota', () => {
  const presentation = buildCodexAccountPresentation(plan, t);
  assert.equal(presentation.displayName, 'Plan account');
  assert.equal(presentation.planLabel, 'ChatGPT 套餐');
  assert.equal(presentation.planClass.includes('free'), false);
  assert.deepEqual(presentation.quotaItems, []);
  assert.equal(getCodexPlanBadgePresentation(plan).label, 'ChatGPT');
  assert.equal(getCodexPlanFilterKey(plan), 'CHATGPT_PLAN');
  assert.equal(isCodexEffectiveFreePlan(plan), false);
  assert.equal(isCodexLocalAccessEligibleAccount(plan, true), true);
  const raw = { ...plan, plan_type: 'Business Special Edition' };
  assert.equal(buildCodexAccountPresentation(raw, t).planLabel, 'Business Special Edition');
  assert.equal(getCodexPlanBadgePresentation(raw).label, 'Business Special Edition');
});

test('plan accounts have their own filter and quota-pool group and never contribute unverified percentages', () => {
  const copiedQuota = { ...plan, quota: { hourly_percentage: 100, weekly_percentage: 80 } } as CodexAccount;
  const pool = summarizeCodexQuotaPool([copiedQuota]);
  assert.equal(pool.byPlan.FREE, undefined);
  assert.equal(pool.byPlan.API_KEY, undefined);
  assert.equal(pool.byPlan.CHATGPT_PLAN.count, 1);
  assert.deepEqual(pool.byPlan.CHATGPT_PLAN.windows, []);
  assert.deepEqual(pool.all.windows, []);
  assert.deepEqual(buildCodexAccountPresentation(copiedQuota, t).quotaItems, []);
  const counts = createCodexPlanFilterCounts(1);
  incrementCodexPlanFilterCount(counts, getCodexPlanFilterKey(plan));
  const options = buildCodexPlanFilterOptions(counts, { chatgptPlanLabel: t('chatgptPlan.title') });
  assert.equal(options.find((option) => option.value === 'CHATGPT_PLAN')?.label, 'ChatGPT 套餐 (1)');
  assert.equal(options.find((option) => option.value === 'FREE')?.label, 'FREE (0)');
  const base = { accounts: [plan], groups: [], searchQuery: '', tagFilter: [], groupFilter: [], activeGroupId: null, resolveDisplayName: (account: CodexAccount) => account.email, compareAccounts: () => 0 };
  assert.deepEqual(filterAndSortCodexOverviewAccounts({ ...base, filterTypes: ['FREE'] }), []);
  assert.deepEqual(filterAndSortCodexOverviewAccounts({ ...base, filterTypes: ['CHATGPT_PLAN'] }), [plan]);
});


test('official subscriptions retain their plain labels and the normal plan styling', () => {
  for (const [raw, css] of [['plus', 'plus codex-plus'], ['TEAM', 'team'], ['pro', 'pro codex-pro-max']]) {
    const official = { ...plan, plan_type: raw };
    const presentation = buildCodexAccountPresentation(official, t);
    assert.equal(presentation.planLabel, raw);
    assert.equal(presentation.planClass, css);
    assert.deepEqual(presentation.quotaItems, []);
    assert.equal(presentation.planLabel.includes('ChatGPT 套餐'), false);
  }
});


test('verified official member uses original quota metrics and subscription without mutating its endpoint', () => {
  const source = { id: 'source', email: 'user@example.com', plan_type: 'team',
    quota: { hourly_percentage: 42, weekly_percentage: 73, hourly_reset_time: 2000000000 },
    tokens: { access_token: 'source-secret', id_token: '' } } as CodexAccount;
  const endpoint = { ...plan, api_wire_api: 'responses', api_base_url: 'https://api.openai.com/v1', requires_reauth: true } as CodexAccount;
  const grant = { id: 'registration', source_account_id: source.id, source_binding_valid: true } as ChatGptPlanAccount;
  const row = projectCodexServiceAccountDisplays([source, endpoint], [grant])[1];
  const result = buildCodexAccountPresentation(row, t);
  assert.equal(result.displayName, buildCodexAccountPresentation(source, t).displayName);
  assert.deepEqual(result.quotaItems, buildCodexAccountPresentation(source, t).quotaItems);
  assert.equal(result.planLabel, buildCodexAccountPresentation(source, t).planLabel);
  assert.equal(result.id, endpoint.id);
  assert.equal(row.quota, undefined);
  assert.equal(row.requires_reauth, true);
  const invalid = projectCodexServiceAccountDisplays([source, row], [{ ...grant, source_binding_valid: false, source_binding_error: 'mismatch' }])[1];
  assert.deepEqual(buildCodexAccountPresentation(invalid, t).quotaItems, []);
});
