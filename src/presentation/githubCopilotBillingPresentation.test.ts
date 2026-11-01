import assert from 'node:assert/strict';
import test from 'node:test';
import { buildGitHubCopilotAccountPresentation } from './platformAccountPresentation.ts';
import type { GitHubCopilotAccount } from '../types/githubCopilot.ts';

const account = {
  id: 'fixture', github_login: 'fixture', github_id: 1,
  github_access_token: '', copilot_token: '', copilot_plan: 'individual',
  created_at: 0, last_used: 0,
  copilot_quota_snapshots: { premium_models: { entitlement: 100, remaining: 60, percent_remaining: 60 } },
} satisfies GitHubCopilotAccount;
const translate = (key: string, options?: unknown): string =>
  key === 'githubCopilot.usage.usedOfTotal' ? `${(options as { used: string }).used} / ${(options as { total: string }).total}` : key;

test('usage billing keeps utilization without calling billing units requests', () => {
  const presentation = buildGitHubCopilotAccountPresentation({ ...account, copilot_token_based_billing: true }, translate);
  const premium = presentation.quotaItems.find(item => item.key === 'premium')!;
  assert.equal(premium.label, 'common.shared.quota.usage');
  assert.notEqual(premium.valueText, '40 / 100');
  assert.equal(premium.percentage, 40);
});

test('legacy and unknown billing retain request-based presentation', () => {
  for (const billing of [false, undefined]) {
    const presentation = buildGitHubCopilotAccountPresentation({ ...account, copilot_token_based_billing: billing }, translate);
    const premium = presentation.quotaItems.find(item => item.key === 'premium')!;
    assert.equal(premium.label, 'githubCopilot.columns.premium');
    assert.equal(premium.valueText, '40 / 100');
  }
});
