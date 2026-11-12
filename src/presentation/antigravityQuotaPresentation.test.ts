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
  getAntigravityModelQuotaDisplayItems,
  getAntigravityQuotaDisplayItems,
  isAccountNeedsReauth,
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
  assert.match(windows, /Claude \(5h\).*?>100%<\/span>/);
  assert.match(windows, /Claude \(Weekly\).*?>0%<\/span>/);
  assert.match(detail, /Claude Sonnet.*0%.*Gemini Pro High.*60%.*Gemini Pro Low.*90%/);
  assert.doesNotMatch(detail, /Weekly|3p-5h/);
  assert.match(renderToStaticMarkup(createElement(AntigravityQuotaSection, { items: [], modelItems, t })), /Gemini Pro Low/);
});

test('explicit quota buckets keep percentages and distant reset times unchanged', () => {
  const items = getAntigravityQuotaDisplayItems(account([
    model('3p-5h', 0), model('3p-weekly', 42),
    model('gemini-5h', 100), model('gemini-weekly', 0),
    model('gemini-argon-wtu-weekly', 91),
  ]), []);
  assert.deepEqual(items.map(({ key, label, percentage, resetTime }) => ({ key, label, percentage, resetTime })), [
    { key: 'claude:5h', label: 'Claude (5h)', percentage: 0, resetTime: futureReset },
    { key: 'claude:weekly', label: 'Claude (Weekly)', percentage: 42, resetTime: futureReset },
    { key: 'gemini:5h', label: 'Gemini (5h)', percentage: 100, resetTime: futureReset },
    { key: 'gemini:weekly', label: 'Gemini (Weekly)', percentage: 0, resetTime: futureReset },
    { key: 'argon:weekly', label: 'Gemini 4 Argon (Weekly)', percentage: 91, resetTime: futureReset },
  ]);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.equal((html.match(/class="codex-quota-mini"/g) || []).length, 5);
  assert.match(html, /Gemini 4 Argon \(Weekly\)/);
  assert.match(html, /91%/);
  const otherAccount = account([model('gemini-5h', 0), model('gemini-weekly', 35)]);
  assert.deepEqual(getAntigravityQuotaDisplayItems(otherAccount, []).map(({ key, percentage }) => [key, percentage]), [
    ['gemini:5h', 0], ['gemini:weekly', 35],
  ]);
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

test('free tier accounts show weekly quota without 5h rows', () => {
  const freeAccount = account([
    model('gemini-weekly', 100),
    model('claude-sonnet', 80),
    model('argon:weekly', 95),
  ]);
  freeAccount.quota!.subscription_tier = 'FREE';
  const items = getAntigravityQuotaDisplayItems(freeAccount, []);
  assert.deepEqual(items.map((i) => i.key), ['claude:weekly', 'gemini:weekly', 'argon:weekly']);
  assert.equal(items.some((i) => i.key.endsWith(':5h')), false);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /Weekly/);
  assert.doesNotMatch(html, /\(5h\)/);
});

test('Ultra model quota and short Argon windows never fill the Argon weekly row', () => {
  const models = [
    model('gemini-4-argon-high', 91),
    model('gemini-argon-5h', 88),
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
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /25%/);
  assert.match(html, /60%/);
  assert.match(html, /0%/);
  assert.doesNotMatch(html, /Gemini 4 Argon \(Weekly\)|91%|88%/);
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

