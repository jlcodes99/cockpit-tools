import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { WorkbuddyAccount } from '../../types/workbuddy';
import { CodexWorkbuddyCredits } from './CodexWorkbuddyCredits';

const i18n = createInstance();
void i18n.init({ lng: 'en', initImmediate: false, resources: { en: { translation: {
  workbuddy: { credits: { total: 'Remaining credits' } },
  common: { loading: 'Loading', automation: { accountNotFound: 'Source account not found' }, shared: { quota: {
    noData: 'No quota data', queryFailed: 'Query failed', unlimited: 'Unlimited', cachedRefreshFailed: 'Refresh failed; cached data',
    lastSuccessfulQuery: 'Last successful: {{time}}',
  } } },
} } } });
const source = (left: number): WorkbuddyAccount => ({
  id: 'source', email: '', access_token: 'not-for-rendering', created_at: 0, last_used: 0,
  usage_updated_at: 1_800_000_000,
  quota_raw: { userResource: { data: { resourceSummary: { Packages: [{ CycleRemainCapacity: String(left) }] } } } },
});
const render = (props: Parameters<typeof CodexWorkbuddyCredits>[0]) => renderToStaticMarkup(
  createElement(I18nextProvider, { i18n }, createElement(CodexWorkbuddyCredits, props)),
);

test('card and compact rows show precise WorkBuddy credits, not dollars or Codex windows', () => {
  for (const compact of [false, true]) {
    const html = render({ source: source(1234.56), compact });
    assert.match(html, /Remaining credits/);
    assert.match(html, /1,234\.56/);
    assert.match(html, /Last successful:/);
    assert.doesNotMatch(html, /not-for-rendering|\$|%|5h|weekly/i);
  }
});

test('a real zero balance remains distinguishable from absent or loading data', () => {
  assert.match(render({ source: source(0) }), /<strong>0<\/strong>/);
  assert.match(render({}), /No quota data/);
  assert.match(render({}), /Source account not found/);
  assert.doesNotMatch(render({}), /<strong>0<\/strong>/);
  assert.match(render({ loading: true }), /Loading/);
  assert.doesNotMatch(render({ loading: true }), /Source account not found/);
});

test('refresh/read failures retain and label the cached balance', () => {
  for (const props of [
    { source: { ...source(15.75), quota_query_last_error: 'private diagnostic' } },
    { source: source(15.75), readFailed: true },
  ]) {
    const html = render(props);
    assert.match(html, /15\.75/);
    assert.match(html, /Refresh failed; cached data/);
    assert.doesNotMatch(html, /private diagnostic/);
  }
  assert.match(render({ readFailed: true }), /Query failed/);
});

test('enterprise balances and unlimited plans use the existing official quota model', () => {
  const enterprise = (limitNum: number) => ({
    ...source(100), enterprise_id: 'enterprise', quota_raw: undefined,
    usage_raw: { code: 0, data: { limitNum, usedNum: 250 } },
  });
  assert.match(render({ source: enterprise(1000) }), /<strong>750<\/strong>/);
  assert.match(render({ source: enterprise(-1) }), /<strong>Unlimited<\/strong>/);
  assert.doesNotMatch(render({ source: enterprise(-1) }), /<strong>-1<\/strong>/);
});

test('invalid refresh timestamps cannot crash an account row', () => {
  for (const usage_updated_at of [NaN, Infinity, -1, Number.MAX_VALUE]) {
    assert.doesNotThrow(() => render({ source: { ...source(1), usage_updated_at } }));
    assert.doesNotMatch(render({ source: { ...source(1), usage_updated_at } }), /Last successful:/);
  }
});

test('unknown credits and enterprise quotas are never invented from personal data', () => {
  const missing = { ...source(100), quota_raw: undefined };
  assert.match(render({ source: missing }), /No quota data/);
  assert.match(render({ source: { ...source(100), enterprise_id: 'enterprise' } }), /No quota data/);
  assert.doesNotMatch(render({ source: { ...source(100), enterprise_id: 'enterprise' } }), /<strong>100<\/strong>/);
});
