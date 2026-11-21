import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { WorkbuddyAccount } from '../../types/workbuddy';
import { PACKAGE_CODE as code } from '../../types/codebuddy-suite.ts';
import { WorkbuddyExpiringCreditsBadge } from './WorkbuddyExpiringCreditsBadge';

// 只保留断言需要的文案，避免测试依赖完整语言包的结构。
const resources = {
  workbuddy: {
    credits: {
      benefit: 'Benefit credits',
      addon: 'Add-on credits',
      expiringSoon: '{{amount}} expiring in 24h',
      expiringSoonTitle: 'Credits expiring within 24 hours',
      expiringSoonSummary: 'Total {{amount}} · {{packageCount}} packages',
      expiresAt: 'Expires {{time}}',
    },
  },
};
const i18n = createInstance();
void i18n.init({ resources: { en: { translation: resources } }, lng: 'en', initImmediate: false });

const now = Date.now();
const at = (hours: number) => new Date(now + hours * 3_600_000).toISOString();
const resource = (PackageCode: string, left: number, expiry: string) => ({
  PackageCode,
  Status: 0,
  CycleCapacitySizePrecise: '100',
  CycleCapacityRemainPrecise: String(left),
  DeductionEndTime: expiry,
});
const account = (resources: object[]) => ({ usage_raw: { data: { resources } } }) as WorkbuddyAccount;
const render = (value: WorkbuddyAccount) => renderToStaticMarkup(
  createElement(I18nextProvider, { i18n }, createElement(WorkbuddyExpiringCreditsBadge, { account: value })),
);

test('expiring credits badge stays hidden when nothing expires within the horizon', () => {
  assert.equal(render(account([resource(code.proMon, 500, at(400))])), '');
  assert.equal(render(account([resource(code.activity, 100, at(30))])), '');
  assert.equal(render(account([resource(code.activity, 0, at(1))])), '');
});

test('expiring credits badge renders the horizon total on the card', () => {
  const markup = render(account([
    resource(code.activity, 70, at(3)),
    resource(code.extra, 45, at(20)),
  ]));
  assert.match(markup, /class="workbuddy-expiring-badge"/);
  assert.match(markup, /115 expiring in 24h/);
  // 浮层只在悬停/聚焦时挂载，静态渲染不应出现。
  assert.doesNotMatch(markup, /workbuddy-expiring-tooltip/);
});
