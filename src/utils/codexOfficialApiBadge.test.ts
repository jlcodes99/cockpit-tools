import assert from 'node:assert/strict';
import test from 'node:test';
import {
  CODEX_OFFICIAL_API_LABEL,
  getCodexPlanBadgePresentation,
  getCodexPlanFilterKey,
  isCodexOfficialApiAccount,
  type CodexAccount,
} from '../types/codex';
import { buildCodexAccountPresentation } from '../presentation/platformAccountPresentation';
import { buildCodexPlanFilterOptions, createCodexPlanFilterCounts, incrementCodexPlanFilterCount } from './codexAccountOverview';

const account = (overrides: Partial<CodexAccount> = {}): CodexAccount => ({
  id: 'official-fixture', email: 'api.openai.com', auth_mode: 'apikey',
  api_provider_mode: 'custom', plan_type: 'API_KEY',
  tokens: { id_token: '', access_token: '' }, created_at: 1, last_used: 1,
  ...overrides,
});

test('official API badges use the effective endpoint and leave account data unchanged', () => {
  for (const overrides of [
    { api_provider_mode: 'openai_builtin' as const },
    { api_base_url: 'https://api.openai.com/v1' },
    { api_base_url: 'https://API.OPENAI.COM:443/v1/' },
    { api_base_url: 'https://api.openai.com', api_provider_name: 'Custom name' },
  ]) {
    const value = account(overrides);
    const before = structuredClone(value);
    assert.equal(isCodexOfficialApiAccount(value), true);
    assert.deepEqual(getCodexPlanBadgePresentation(value), {
      label: CODEX_OFFICIAL_API_LABEL, className: 'api-key codex-official-api',
    });
    assert.equal(getCodexPlanBadgePresentation(value, { preserveRawNonProLabel: true }).label, CODEX_OFFICIAL_API_LABEL);
    assert.equal(getCodexPlanFilterKey(value), CODEX_OFFICIAL_API_LABEL);
    const shared = buildCodexAccountPresentation(value, (key: string) => key);
    assert.equal(shared.planLabel, CODEX_OFFICIAL_API_LABEL);
    assert.match(shared.planClass!, /\bcodex-official-api\b/);
    assert.deepEqual(value, before);
  }
});

test('provider names, key prefixes, relays and OAuth grants cannot become official API accounts', () => {
  for (const overrides of [
    {},
    { api_provider_mode: 'openai_builtin' as const, api_base_url: 'https://relay.example/v1' },
    { api_provider_name: 'OpenAI Official', api_base_url: 'https://relay.example/v1', openai_api_key: 'sk-synthetic' },
    { api_base_url: 'https://api.openai.com.example/v1' },
    { api_base_url: 'https://api.openai.com@relay.example/v1' },
    { api_base_url: 'http://api.openai.com/v1' },
    { api_base_url: 'https://api.openai.com/v1?proxy=1' },
    { api_base_url: 'https://api.openai.com/v1', auth_mode: 'oauth', plan_type: 'plus' },
    { api_base_url: 'https://api.openai.com/v1', api_provider_id: 'cockpit_chatgpt_plan' },
  ]) {
    const value = account(overrides);
    assert.equal(isCodexOfficialApiAccount(value), false, JSON.stringify(overrides));
    assert.notEqual(getCodexPlanBadgePresentation(value).label, CODEX_OFFICIAL_API_LABEL);
  }
});

test('the overview counts and filters official API separately from third-party API keys', () => {
  const values = [account({ api_provider_mode: 'openai_builtin' }), account({ api_base_url: 'https://relay.example/v1' })];
  const counts = createCodexPlanFilterCounts(values.length);
  for (const value of values) incrementCodexPlanFilterCount(counts, getCodexPlanFilterKey(value));
  const options = buildCodexPlanFilterOptions(counts);
  assert.deepEqual(options.find(item => item.value === CODEX_OFFICIAL_API_LABEL), { value: CODEX_OFFICIAL_API_LABEL, label: '官API (1)' });
  assert.deepEqual(options.find(item => item.value === 'API_KEY'), { value: 'API_KEY', label: 'API_KEY (1)' });
});
