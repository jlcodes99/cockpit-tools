import assert from 'node:assert/strict';
import test from 'node:test';

import {
  ALL_CODEX_AUTO_REFRESH_PLAN_KEYS,
  buildCodexAutoRefreshPlanOptions,
  isCodexAccountEligibleForAutomaticQuotaRefresh,
  resolveCodexAutoRefreshPlanKey,
  sanitizeCodexAutoRefreshPlanKeys,
} from './codexAutoRefreshPlanScope.ts';

test('normalizes known Codex plan spellings without treating a missing plan as Free', () => {
  assert.equal(resolveCodexAutoRefreshPlanKey('chatgpt-enterprise'), 'enterprise');
  assert.equal(resolveCodexAutoRefreshPlanKey('BUSINESS'), 'business');
  assert.equal(resolveCodexAutoRefreshPlanKey('team_plan'), 'team');
  assert.equal(resolveCodexAutoRefreshPlanKey('edu-k12'), 'edu_k12');
  assert.equal(resolveCodexAutoRefreshPlanKey('pro_lite'), 'pro');
  assert.equal(resolveCodexAutoRefreshPlanKey('plus'), 'plus');
  assert.equal(resolveCodexAutoRefreshPlanKey(undefined), 'unknown');
  assert.equal(resolveCodexAutoRefreshPlanKey('google'), 'unknown');
});

test('filters only standard OAuth plans while preserving New API behavior', () => {
  const oauthAccount = {
    id: 'oauth',
    email: 'oauth@example.com',
    plan_type: 'free',
    tokens: {
      id_token: '',
      access_token: 'oauth-access',
      refresh_token: 'oauth-refresh',
    },
    created_at: 0,
    last_used: 0,
  };
  assert.equal(
    isCodexAccountEligibleForAutomaticQuotaRefresh(oauthAccount, ['plus']),
    false,
  );
  assert.equal(
    isCodexAccountEligibleForAutomaticQuotaRefresh(oauthAccount, ['free']),
    true,
  );

  assert.equal(
    isCodexAccountEligibleForAutomaticQuotaRefresh(
      {
        id: 'new-api',
        email: 'new-api',
        auth_mode: 'apikey',
        api_provider_id: 'new_api',
        plan_type: 'API_KEY',
        tokens: { id_token: '', access_token: '' },
        created_at: 0,
        last_used: 0,
      },
      [],
    ),
    true,
  );
});

test('defaults a missing selection to all plans but preserves an explicit empty selection', () => {
  assert.deepEqual(
    sanitizeCodexAutoRefreshPlanKeys(undefined),
    ALL_CODEX_AUTO_REFRESH_PLAN_KEYS,
  );
  assert.deepEqual(sanitizeCodexAutoRefreshPlanKeys([]), []);
  assert.deepEqual(
    sanitizeCodexAutoRefreshPlanKeys(['pro', 'invalid', 'plus', 'pro']),
    ['plus', 'pro'],
  );
});

test('builds stable options and classifies missing or unrecognized plans as unknown', () => {
  const options = buildCodexAutoRefreshPlanOptions([
    { plan_type: 'free' },
    { plan_type: 'PRO_MAX' },
    { plan_type: undefined },
    { plan_type: 'custom-plan' },
  ]);
  const counts = Object.fromEntries(options.map((option) => [option.key, option.count]));

  assert.equal(counts.free, 1);
  assert.equal(counts.pro, 1);
  assert.equal(counts.unknown, 2);
});
