import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { TFunction } from 'i18next';
import type { Account, ModelQuota } from '../types/account';
import { AntigravityQuotaSection } from '../components/AntigravityQuotaSection';
import { buildAccountTierCounts, buildAccountTierFilterOptions } from '../utils/accountFilters';
import {
  buildAntigravityAccountPresentation,
  buildQuotaPreviewLines,
  getAntigravityModelQuotaDisplayItems,
  getAntigravityQuotaDisplayItems,
  isAccountNeedsReauth,
} from './platformAccountPresentation';

const t = ((key: string, defaultValue?: string | Record<string, unknown>, vars?: Record<string, unknown>) => {
  const base = typeof defaultValue === 'string' ? defaultValue : key;
  const params = typeof defaultValue === 'object' && defaultValue !== null ? defaultValue : vars;
  return params
    ? base.replace(/\{\{(\w+)\}\}/g, (_, k) => (k in params ? String(params[k]) : `{{${k}}}`))
    : base;
}) as unknown as TFunction;
const futureReset = new Date(Date.now() + 6 * 60 * 60 * 1000).toISOString();
function account(models: ModelQuota[], stale = false): Account {
  return {
    id: 'test', email: 'test@example.com', created_at: 0, last_used: 0,
    token: { access_token: '', refresh_token: '', expires_in: 0, expiry_timestamp: 0, token_type: '' },
    quota: { models, last_updated: 0, quota_summary_stale: stale },
  };
}
const model = (name: string, percentage: number): ModelQuota => ({ name, percentage, reset_time: futureReset });

test('missing Antigravity plans use the neutral badge and remain filterable', () => {
  for (const subscriptionTier of [undefined, ' ', ' UNKNOWN ']) {
    const fixture = account([]);
    fixture.quota!.subscription_tier = subscriptionTier;
    const presentation = buildAntigravityAccountPresentation(fixture, [], t);
    assert.equal(presentation.planLabel, '—');
    assert.equal(presentation.planClass, 'unknown');
    const counts = buildAccountTierCounts([fixture], {});
    assert.equal(counts.UNKNOWN, 1);
    assert.equal(counts.FREE, 0);
    const filter = buildAccountTierFilterOptions((key) => key === 'common.unknown' ? '未知' : key, counts)
      .find(({ value }) => value === 'UNKNOWN');
    assert.equal(filter?.label, '未知 (1)');
  }

  const fixture = account([]);
  fixture.quota!.subscription_tier = 'g1-pro-tier';
  const presentation = buildAntigravityAccountPresentation(fixture, [], t);
  assert.equal(presentation.planLabel, 'g1-pro-tier');
  assert.equal(presentation.planClass, 'pro');
});

test('list disclosure shows each real model and keeps account-wide windows separate', () => {
  const fixture = account([
    model('3p-5h', 100),
    model('3p-weekly', 0),
    model('gemini-argon-wtu-weekly', 25),
    { ...model('claude-sonnet', 0), display_name: 'Claude Sonnet' },
    { ...model('gemini-pro-high', 60), display_name: 'Gemini Pro High' },
    { ...model('gemini-pro-low', 90), display_name: 'Gemini Pro Low' },
  ]);
  const items = getAntigravityQuotaDisplayItems(fixture, []);
  const modelItems = getAntigravityModelQuotaDisplayItems(fixture);
  assert.deepEqual(modelItems.map(({ label, percentage, resetTime }) => [label, percentage, resetTime]), [
    ['Claude Sonnet', 0, futureReset],
    ['Gemini Pro High', 60, futureReset],
    ['Gemini Pro Low', 90, futureReset],
  ]);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, modelItems, t }));
  const [windows, detail] = html.split('<details');
  assert.match(windows, /Claude · 5h.*?100%/);
  assert.match(windows, /Claude · Weekly.*?0%/);
  assert.doesNotMatch(windows, /Gemini 4 Argon/);
  assert.match(detail, /Claude Sonnet.*0%.*Gemini Pro High.*60%.*Gemini Pro Low.*90%/);
  assert.doesNotMatch(detail, /Weekly|3p-5h|gemini-argon-wtu-weekly/);
  assert.match(renderToStaticMarkup(createElement(AntigravityQuotaSection, { items: [], modelItems, t })), /Gemini Pro Low/);
});

test('explicit quota buckets keep percentages and distant reset times unchanged', () => {
  const items = getAntigravityQuotaDisplayItems(account([
    model('3p-5h', 0), model('3p-weekly', 42),
    model('gemini-5h', 100), model('gemini-weekly', 0),
    model('gemini-carbon-wtu-weekly', 91),
    model('gemini-argon-wtu-weekly', 77),
  ]), []);
  assert.deepEqual(items.map(({ key, label, percentage, resetTime }) => ({ key, label, percentage, resetTime })), [
    { key: 'claude:5h', label: 'Claude (5h)', percentage: 0, resetTime: futureReset },
    { key: 'claude:weekly', label: 'Claude (Weekly)', percentage: 42, resetTime: futureReset },
    { key: 'gemini:5h', label: 'Gemini (5h)', percentage: 100, resetTime: futureReset },
    { key: 'gemini:weekly', label: 'Gemini (Weekly)', percentage: 0, resetTime: futureReset },
    { key: 'carbon:weekly', label: 'Gemini 4 Carbon (Weekly)', percentage: 91, resetTime: futureReset },
  ]);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.equal((html.match(/class="ag-quota-cell /g) || []).length, 5);
  assert.match(html, /Gemini 4 Carbon \(Weekly\)/);
  assert.doesNotMatch(html, /Gemini 4 Argon \(Weekly\)/);
  assert.match(html, /91%/);
  const otherAccount = account([model('gemini-5h', 0), model('gemini-weekly', 35)]);
  assert.deepEqual(getAntigravityQuotaDisplayItems(otherAccount, []).map(({ key, percentage }) => [key, percentage]), [
    ['gemini:5h', 0], ['gemini:weekly', 35],
  ]);
});

test('a confirmed exhausted Claude week makes the short window inactive across quota views', () => {
  const weekly = { ...model('3p-weekly', 0), summary_exhausted: true };
  const fixture = account([model('3p-5h', 63), weekly, model('gemini-5h', 100), model('gemini-weekly', 86)]);
  fixture.quota!.subscription_tier = 'PRO';
  const items = getAntigravityQuotaDisplayItems(fixture, []);
  for (const isList of [false, true]) {
    const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, isList, t }));
    assert.match(html, /Claude · 5h.*?weeklyLimitReached/);
    assert.doesNotMatch(html, /63%/);
    assert.match(html, /Claude · Weekly.*?0%/);
    assert.match(html, /Gemini · 5h.*?100%/);
  }
  const preview = buildQuotaPreviewLines(buildAntigravityAccountPresentation(fixture, [], t).quotaItems);
  assert.match(preview[0].title, /weeklyLimitReached/);
  assert.doesNotMatch(preview[0].text, /63%/);
  assert.equal(fixture.quota!.models[0].percentage, 63);

  for (const [bucket, stale] of [
    [{ ...weekly, summary_exhausted: false }, false],
    [model('3p-weekly', 0), false],
    [weekly, true],
    [{ ...weekly, reset_time: '2020-01-01T00:00:00Z' }, false],
  ] as const) {
    const possible = account([model('3p-5h', 63), bucket], stale);
    const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items: getAntigravityQuotaDisplayItems(possible, []), t }));
    assert.match(html, /Claude · 5h.*?63%/);
    assert.doesNotMatch(html, /weeklyLimitReached/);
  }
});

test('known quota summaries keep exhausted and unknown Gemini windows distinct from model percentages', () => {
  const models = [model('gemini-pro-high', 100), model('gemini-pro-low', 80)];
  const current = account([...models, model('gemini-weekly', 0)]);
  current.quota!.quota_summary_updated_at = 1;
  const stale = account([...models, model('gemini-weekly', 0)], true);
  for (const fixture of [current, stale]) {
    assert.deepEqual(getAntigravityQuotaDisplayItems(fixture, []).map(({ key, percentage }) => [key, percentage]), [
      ['gemini:weekly', 0],
    ]);
    assert.deepEqual(getAntigravityModelQuotaDisplayItems(fixture).map(({ percentage }) => percentage), [100, 80]);
  }
  assert.deepEqual(getAntigravityQuotaDisplayItems(account(models), []).map(({ key, percentage }) => [key, percentage]), [
    ['gemini:5h', 100], ['gemini:weekly', 80],
  ]);
});

test('Carbon has a separate weekly quota and keeps its model-level rows', () => {
  const ultra = account([
    model('gemini-4-carbon-high', 7),
    model('gemini-4-hydrogen-high', 39),
    model('gemini-carbon-wtu-weekly', 21),
  ]);
  ultra.quota!.subscription_tier = 'ULTRA';
  const items = getAntigravityQuotaDisplayItems(ultra, []);
  assert.deepEqual(items.map(({ key, percentage }) => [key, percentage]), [
    ['gemini:5h', 39], ['carbon:weekly', 21],
  ]);
  const modelItems = getAntigravityModelQuotaDisplayItems(ultra);
  assert.deepEqual(modelItems.map(({ key }) => key), [
    'gemini-4-carbon-high', 'gemini-4-hydrogen-high',
  ]);
  assert.deepEqual(modelItems.map(({ label }) => label), [
    'Gemini 4 Carbon (High)', 'Gemini 4 Hydrogen (High)',
  ]);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, modelItems, t }));
  assert.match(html, /Gemini 4 Carbon \(Weekly\).*?21%/);
});

test('free tier accounts show weekly quota without 5h rows', () => {
  const freeAccount = account([
    model('gemini-weekly', 100),
    model('claude-sonnet', 80),
    model('carbon:weekly', 95),
    model('argon:weekly', 95),
  ]);
  freeAccount.quota!.subscription_tier = 'FREE';
  const items = getAntigravityQuotaDisplayItems(freeAccount, []);
  assert.deepEqual(items.map((i) => i.key), ['claude:weekly', 'gemini:weekly', 'carbon:weekly']);
  assert.equal(items.some((i) => i.key.endsWith(':5h')), false);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /Weekly/);
  assert.doesNotMatch(html, /· 5h/);
  assert.doesNotMatch(html, /Gemini 4 Argon/);
});

test('free accounts show cached weekly buckets separately from current model percentages', () => {
  const free = account([
    model('3p-weekly', 28), model('gemini-pro', 100), model('claude-sonnet', 100),
  ], true);
  free.quota!.subscription_tier = 'FREE';
  const items = getAntigravityQuotaDisplayItems(free, []);
  assert.deepEqual(items.map(({ key, percentage, stale }) => [key, percentage, stale]), [
    ['claude:weekly', 28, true],
  ]);
  assert.deepEqual(getAntigravityModelQuotaDisplayItems(free).map(({ percentage }) => percentage), [100, 100]);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /cachedRefreshFailed/);
  assert.doesNotMatch(html, /100%/);
});

test('removed Argon quota pool and legacy Argon buckets never render a weekly row or model row', () => {
  const models = [
    model('gemini-4-argon-high', 91),
    model('gemini-argon-5h', 88),
    model('gemini-argon-wtu-weekly', 77),
    model('argon:weekly', 66),
    model('gemini-3.1-pro-high', 25),
    model('gemini-3.1-pro-low', 60),
    model('claude-sonnet', 0),
  ];
  const ultra = account(models);
  ultra.quota!.subscription_tier = 'ULTRA';
  const items = getAntigravityQuotaDisplayItems(ultra, []);
  assert.deepEqual(items.map((item) => [item.key, item.percentage]), [
    ['claude:5h', 0],
    ['gemini:5h', 25],
    ['gemini:weekly', 60],
  ]);
  const modelItems = getAntigravityModelQuotaDisplayItems(ultra);
  assert.deepEqual(modelItems.map(({ key }) => key), [
    'gemini-4-argon-high', 'gemini-3.1-pro-high', 'gemini-3.1-pro-low', 'claude-sonnet',
  ]);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /25%/);
  assert.match(html, /60%/);
  assert.match(html, /0%/);
  assert.doesNotMatch(html, /Gemini 4 Argon \(Weekly\)|91%|88%|77%|66%/);
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

test('Carbon quota stays separate from shared Gemini quota and model details', () => {
  const fixture = account([
    model('gemini-4-carbon-high', 90),
    model('gemini-carbon-wtu-weekly', 22),
    model('gemini-4-hydrogen-high', 65),
  ]);
  assert.deepEqual(getAntigravityQuotaDisplayItems(fixture, []).map(({ key, percentage }) => [key, percentage]), [
    ['gemini:5h', 65], ['carbon:weekly', 22],
  ]);
  assert.deepEqual(getAntigravityModelQuotaDisplayItems(fixture).map(({ key }) => key), [
    'gemini-4-carbon-high', 'gemini-4-hydrogen-high',
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

test('isAccountNeedsReauth accurately identifies re-authorization required accounts', () => {
  const normalAcc = account([model('claude:weekly', 100)]);
  assert.equal(isAccountNeedsReauth(normalAcc), false);

  const disabledReasonAcc = { ...normalAcc, disabled_reason: 'verification_required' };
  assert.equal(isAccountNeedsReauth(disabledReasonAcc), true);

  const invalidGrantAcc = { ...normalAcc, disabled_reason: 'invalid_grant: token revoked' };
  assert.equal(isAccountNeedsReauth(invalidGrantAcc), true);

  const quotaErrorReasonAcc: Account = {
    ...normalAcc,
    quota_error: { code: 403, message: 'Verify your account to continue.', reason: 'VALIDATION_REQUIRED', timestamp: 123 },
  };
  assert.equal(isAccountNeedsReauth(quotaErrorReasonAcc), true);

  const validationUrlAcc: Account = {
    ...normalAcc,
    quota_error: { code: 403, message: 'Verify', validation_url: 'https://accounts.google.com/signin/continue', timestamp: 123 },
  };
  assert.equal(isAccountNeedsReauth(validationUrlAcc), true);
});

test('AntigravityQuotaSection renders cachedNeedsReauth warning when isNeedsReauth is true', () => {
  const cached = account([model('gemini-5h', 25)], true);
  const items = getAntigravityQuotaDisplayItems(cached, []);

  // When isNeedsReauth is true, displays cachedNeedsReauth
  const htmlReauth = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, isNeedsReauth: true, t }));
  assert.match(htmlReauth, /⚠️ 账号需完成网页验证以继续使用/);
  assert.match(htmlReauth, /quota-reauth-warning/);

  // When isNeedsReauth is false, displays cachedRefreshFailed
  const htmlNormal = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, isNeedsReauth: false, t }));
  assert.match(htmlNormal, /common\.shared\.quota\.cachedRefreshFailed/);
  assert.doesNotMatch(htmlNormal, /quota-reauth-warning/);
});

test('Antigravity card time picker can show the quota refresh time without selecting the card', async () => {
  const { loadHookModule } = await import('../../tests/helpers/reactHookHarness');
  const h = loadHookModule(new URL('../components/AccountLastUsed.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (_key: string, fallback?: string) => fallback || _key }) },
    '../stores/useUiConfigStore': {
      useUiConfigStore: (selector: (state: unknown) => unknown) =>
        selector({ timeDisplayMode: 'refresh', cardTimeModes: new Map(), setCardTimeMode() {} }),
    },
    './SingleSelectDropdown': { SingleSelectDropdown: () => null },
  });
  const rendered = h.render(() =>
    h.exports.AccountLastUsed({
      accountId: 'acc-1',
      lastSwitchedAt: 1700000000,
      refreshAt: 1700000300,
      createdAt: 1700000000,
      formatDate: String,
    }),
  );
  const picker = rendered.props.children[1];
  assert.equal(picker.props.value, 'refresh');
  assert.equal(picker.props.options[0].disabled, false);
  let stopped = 0;
  rendered.props.onClick({ stopPropagation: () => { stopped += 1; } });
  assert.equal(stopped, 1);
  h.unmount();
});
