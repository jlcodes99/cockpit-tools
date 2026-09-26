import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { TFunction } from 'i18next';
import type { Account, ModelQuota } from '../types/account';
import { AntigravityQuotaSection } from '../components/AntigravityQuotaSection';
import {
  buildAntigravityAccountPresentation,
  buildQuotaPreviewLines,
  getAntigravityQuotaDisplayItems,
} from './platformAccountPresentation';

const t = ((key: string, defaultValue?: string | Record<string, unknown>) =>
  typeof defaultValue === 'string' ? defaultValue : key) as unknown as TFunction;
const futureReset = new Date(Date.now() + 6 * 60 * 60 * 1000).toISOString();
function account(models: ModelQuota[], stale = false): Account {
  return {
    id: 'test', email: 'test@example.com', created_at: 0, last_used: 0,
    token: { access_token: '', refresh_token: '', expires_in: 0, expiry_timestamp: 0, token_type: '' },
    quota: { models, last_updated: 0, quota_summary_stale: stale },
  };
}
const model = (name: string, percentage: number): ModelQuota => ({ name, percentage, reset_time: futureReset });

test('explicit quota buckets keep percentages and distant reset times unchanged', () => {
  const items = getAntigravityQuotaDisplayItems(account([
    model('3p-5h', 0), model('3p-weekly', 42),
    model('gemini-5h', 25), model('gemini-weekly', 70),
  ]), []);
  assert.deepEqual(items.map(({ key, percentage, resetTime }) => ({ key, percentage, resetTime })), [
    { key: 'claude:5h', percentage: 0, resetTime: futureReset },
    { key: 'claude:weekly', percentage: 42, resetTime: futureReset },
    { key: 'gemini:5h', percentage: 25, resetTime: futureReset },
    { key: 'gemini:weekly', percentage: 70, resetTime: futureReset },
  ]);
});

test('free tier accounts only map to weekly quota and omit 5h quota bars', () => {
  const freeAccount: Account = {
    ...account([
      model('gemini-weekly', 100),
      model('claude-sonnet', 80),
    ]),
    quota: {
      models: [
        model('gemini-weekly', 100),
        model('claude-sonnet', 80),
      ],
      last_updated: 0,
      subscription_tier: 'FREE',
    },
  };
  const items = getAntigravityQuotaDisplayItems(freeAccount, []);
  assert.deepEqual(items.map((i) => i.key), ['claude:weekly', 'gemini:weekly']);
  assert.equal(items.some((i) => i.key.endsWith(':5h')), false);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /Weekly/);
  assert.doesNotMatch(html, />5h</);
});

test('free account models fallback and map to quota windows on the card', () => {
  const models = [model('gemini-3.1-pro-high', 25), model('gemini-3.1-pro-low', 60), model('claude-sonnet', 0)];
  const items = getAntigravityQuotaDisplayItems(account(models), []);
  assert.equal(items.length, 3);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /25%/);
  assert.match(html, /60%/);
  assert.match(html, /0%/);
  assert.doesNotMatch(html, /quota-empty/);
});

test('only the Claude/Gemini windows are shown, aliases are not duplicated', () => {
  const items = getAntigravityQuotaDisplayItems(account([
    model('gemini:5h', 15), model('gemini-5h', 15),
    { ...model('custom-model', 35), display_name: 'Custom model' },
  ]), []);
  assert.deepEqual(items.map((item) => [item.key, item.label]), [
    ['gemini:5h', 'Gemini (5h)'],
  ]);
});

for (const isList of [false, true]) {
  test(`missing windows render no-data rather than full progress (list=${isList})`, () => {
    const items = getAntigravityQuotaDisplayItems(account([model('gemini-5h', 25)]), []);
    const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, isList, t }));
    assert.match(html, /25%/);
    assert.doesNotMatch(html, /100%/);
    assert.equal((html.match(/>—</g) || []).length, 3);
    assert.equal((html.match(/style="width:/g) || []).length, 1);
    const empty = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items: [], isList, t }));
    assert.match(empty, /overview.noQuotaData/);
    assert.doesNotMatch(empty, /100%/);
  });
}

test('stale summary warns only on cached buckets, including shared presentation and previews', () => {
  const cached = account([model('gemini-5h', 25), model('gemini-pro-high', 40)], true);
  const items = getAntigravityQuotaDisplayItems(cached, []);
  assert.equal(items.length, 1);
  assert.equal(items[0].stale, true);
  const presentation = buildAntigravityAccountPresentation(cached, [], t);
  assert.match(presentation.quotaItems[0].resetText ?? '', /common.shared.quota.cachedRefreshFailed/);
  assert.equal(presentation.quotaItems.length, 1);
  assert.match(buildQuotaPreviewLines(presentation.quotaItems)[0].title, /cachedRefreshFailed/);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /common.shared.quota.cachedRefreshFailed/);
  const noCachedBuckets = getAntigravityQuotaDisplayItems(account([model('custom-model', 40)], true), []);
  const noWarning = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items: noCachedBuckets, t }));
  assert.doesNotMatch(noWarning, /cachedRefreshFailed/);
});
