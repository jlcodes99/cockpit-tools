import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const source = await readFile(
  new URL('../../src/services/modelProviderUsageHelpers.ts', import.meta.url),
  'utf8',
);
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
});
const helpers = await import(
  `data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`
);

const balances = [
  {
    currency: 'CNY',
    totalBalance: '123.4500',
    grantedBalance: '3.2500',
    toppedUpBalance: '120.2000',
  },
  {
    currency: 'USD',
    totalBalance: '17.64',
    grantedBalance: '0.00',
    toppedUpBalance: '17.64',
  },
];

test('DeepSeek currency follows every supported client language', () => {
  assert.equal(helpers.preferredDeepSeekCurrency('zh-CN'), 'CNY');
  assert.equal(helpers.preferredDeepSeekCurrency('zh-tw'), 'CNY');
  for (const language of [
    'en', 'ja', 'es', 'de', 'fr', 'pt-br', 'ru', 'ko', 'it', 'tr', 'pl', 'cs',
    'vi', 'ar', 'id',
  ]) {
    assert.equal(helpers.preferredDeepSeekCurrency(language), 'USD', language);
  }
});

test('DeepSeek currency selection falls back to the first real currency', () => {
  assert.equal(helpers.selectDeepSeekBalanceInfo(balances, 'zh-cn').currency, 'CNY');
  assert.equal(helpers.selectDeepSeekBalanceInfo(balances, 'en').currency, 'USD');
  assert.equal(
    helpers.selectDeepSeekBalanceInfo([balances[0]], 'en').currency,
    'CNY',
  );
});

test('DeepSeek cache round-trip preserves balance infos and availability', () => {
  const cached = helpers.parseCodexApiKeyUsageCache(
    helpers.serializeCodexApiKeyUsageCache({
      account: {
        loading: true,
        updatedAt: 123,
        summary: {
          mode: 'deepseek',
          isAvailable: false,
          balanceInfos: balances,
          modelStatsCount: 0,
          latencyMs: 19,
        },
      },
    }),
  );

  assert.equal(cached.account.loading, false);
  assert.equal(cached.account.summary.isAvailable, false);
  assert.deepEqual(cached.account.summary.balanceInfos, balances);
});
