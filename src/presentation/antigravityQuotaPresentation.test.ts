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

test('explicit quota buckets keep percentages and distant reset times unchanged', () => {
  const items = getAntigravityQuotaDisplayItems(account([
    model('3p-5h', 0), model('3p-weekly', 42),
    model('gemini-5h', 25), model('gemini-weekly', 70),
    model('gemini-argon-wtu-weekly', 91),
  ]), []);
  assert.deepEqual(items.map(({ key, label, percentage, resetTime }) => ({ key, label, percentage, resetTime })), [
    { key: 'claude:5h', label: 'Claude (5h)', percentage: 0, resetTime: futureReset },
    { key: 'claude:weekly', label: 'Claude (Weekly)', percentage: 42, resetTime: futureReset },
    { key: 'gemini:5h', label: 'Gemini (5h)', percentage: 25, resetTime: futureReset },
    { key: 'gemini:weekly', label: 'Gemini (Weekly)', percentage: 70, resetTime: futureReset },
    { key: 'argon:weekly', label: 'Gemini 4 Argon (Weekly)', percentage: 91, resetTime: futureReset },
  ]);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.equal((html.match(/class="codex-quota-mini"/g) || []).length, 5);
  assert.match(html, /Gemini 4 Argon \(Weekly\)/);
  assert.match(html, /91%/);
});

test('free tier accounts only map to weekly quota and omit 5h quota bars', () => {
  const freeAccount: Account = {
    ...account([
      model('gemini-weekly', 100),
      model('claude-sonnet', 80),
      model('gemini-4-argon-high', 95),
    ]),
    quota: {
      models: [
        model('gemini-weekly', 100),
        model('claude-sonnet', 80),
        model('gemini-4-argon-high', 95),
      ],
      last_updated: 0,
      subscription_tier: 'FREE',
    },
  };
  const items = getAntigravityQuotaDisplayItems(freeAccount, []);
  assert.deepEqual(items.map((i) => i.key), ['claude:weekly', 'gemini:weekly', 'argon:weekly']);
  assert.equal(items.some((i) => i.key.endsWith(':5h')), false);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /Weekly/);
  assert.doesNotMatch(html, /\(5h\)/);
});

test('free account models fallback and map to quota windows on the card', () => {
  const models = [
    model('gemini-4-argon-high', 91),
    model('gemini-3.1-pro-high', 25),
    model('gemini-3.1-pro-low', 60),
    model('claude-sonnet', 0),
  ];
  const items = getAntigravityQuotaDisplayItems(account(models), []);
  assert.equal(items.length, 4);
  assert.deepEqual(items.map((item) => [item.key, item.percentage]), [
    ['claude:5h', 0],
    ['gemini:5h', 25],
    ['gemini:weekly', 60],
    ['argon:weekly', 91],
  ]);
  const html = renderToStaticMarkup(createElement(AntigravityQuotaSection, { items, t }));
  assert.match(html, /25%/);
  assert.match(html, /60%/);
  assert.match(html, /0%/);
  assert.match(html, /91%/);
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
    assert.equal((html.match(/>—</g) || []).length, 4);
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

test('Antigravity card time elements prevent overflow occlusion across grid, compact, and quota views', async () => {
  const { readFileSync } = await import('node:fs');
  const { loadHookModule } = await import('../../tests/helpers/reactHookHarness');
  const accountsCss = readFileSync(new URL('../styles/pages/accounts.css', import.meta.url), 'utf8');
  const compactCss = readFileSync(new URL('../styles/CompactView.module.css', import.meta.url), 'utf8');

  // 1. .account-last-used and .alu-select must constrain width and clip overflow
  assert.match(accountsCss, /\.account-last-used\s*\{[^}]*overflow:\s*hidden/s);
  assert.match(accountsCss, /\.account-last-used\s+\.alu-select\s*\{[^}]*min-width:\s*0/s);
  assert.match(accountsCss, /\.account-last-used\s+\.alu-select\s*\{[^}]*text-overflow:\s*ellipsis/s);

  // 2. .card-footer must allow wrapping and .card-actions must align right so time selector never overlaps action buttons
  assert.match(accountsCss, /\.card-footer\s*\{[^}]*flex-wrap:\s*wrap/s);
  assert.match(accountsCss, /\.card-actions\s*\{[^}]*margin-left:\s*auto/s);

  // 3. CompactView .card must wrap and place .account-last-used on its own full-width row
  assert.match(compactCss, /\.card\s*\{[^}]*flex-wrap:\s*wrap/s);
  assert.match(compactCss, /\.card\s+:global\(\.account-last-used\)\s*\{[^}]*flex:\s*1\s+0\s+100%/s);

  // 4. Odd last quota item reset time must not be forced onto row 1 between label and percentage
  assert.doesNotMatch(
    accountsCss,
    /\.codex-quota-mini:last-child:nth-child\(odd\)\s+\.codex-quota-mini-reset\s*\{[^}]*grid-row:\s*1/s,
  );

  // 5. AccountLastUsed stops click propagation so clicking time selector in CompactView does not toggle card selection
  const h = loadHookModule(new URL('../components/AccountLastUsed.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (_key: string, fallback?: string) => fallback || _key }) },
    '../stores/useUiConfigStore': {
      useUiConfigStore: (selector: (s: any) => any) =>
        selector({ timeDisplayMode: 'switch', cardTimeModes: new Map(), setCardTimeMode() {} }),
    },
  });
  let stopped = 0;
  const rendered = h.render(() =>
    h.exports.AccountLastUsed({
      accountId: 'acc-1',
      lastSwitchedAt: 1700000000,
      createdAt: 1700000000,
      formatDate: () => '2026/11/11 09:46',
    }),
  );
  rendered.props.onClick({
    stopPropagation() {
      stopped++;
    },
  });
  assert.equal(stopped, 1);
  h.unmount();

});

