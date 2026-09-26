import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import postcss from 'postcss';
import { getCodexPlanBadgePresentation, type CodexAccount } from '../../../types/codex.ts';
import { withCodexPlanBadgeStyle } from '../../../utils/codexPreferences.ts';
import { pelicanAccountSummary } from './pelicanSetupModel.ts';

test('Pelican subscription badges share overview declarations rather than copy styling', () => {
  const css = postcss.parse(readFileSync(new URL('../../../styles/pages/codex-account-cards.css', import.meta.url), 'utf8'));
  for (const suffix of ['', '::before', '.free', '.plus', '.team', '.enterprise', '.pro', '.codex-plus', '.codex-pro-lite', '.codex-pro-max']) {
    let shared = false;
    css.walkRules((rule) => {
      if (rule.selectors.includes(`.codex-account-card .tier-badge${suffix}`)
        && rule.selectors.includes(`.pelican-account-summary .tier-badge${suffix}`)) shared = true;
    });
    assert(shared, `Missing shared badge rule: ${suffix}`);
  }
  const localCss = readFileSync(new URL('./pelican.css', import.meta.url), 'utf8');
  assert(!localCss.includes('.pelican-plan-badge'));
});

test('overview plan styling and user-selected variants do not alter raw subscription values', () => {
  for (const plan of ['free', 'plus', 'team', 'enterprise', 'pro']) {
    const account = { id: 'test', auth_mode: 'oauth', plan_type: plan } as CodexAccount;
    const summary = pelicanAccountSummary(account);
    assert.equal(summary.planClass, getCodexPlanBadgePresentation(account).className);
    for (const style of ['default', 'outline', 'soft', 'mono'] as const) {
      const className = withCodexPlanBadgeStyle(summary.planClass, style);
      assert.equal(className, style === 'default' ? summary.planClass : `${summary.planClass} plan-badge-style-${style}`);
      assert.equal(summary.plan, plan);
    }
  }
});
