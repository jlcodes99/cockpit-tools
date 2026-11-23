import assert from 'node:assert/strict';
import test from 'node:test';
import { createInstance } from 'i18next';
import { getCodexAccountRefreshFeedback, refreshCodexQuotaWithFeedback } from './codexQuotaRefreshFeedback.ts';

test('account feedback distinguishes completed, skipped, and failed refreshes', async () => {
  const i18n = createInstance();
  await i18n.init({
    lng: 'en',
    resources: {
      en: { translation: {
        common: { shared: { refreshSuccess: 'Refreshed', refreshFailed: 'Refresh failed: {{error}}' } },
        codex: { proxy: { runtimeBusy: 'Proxy is busy' } },
      } },
    },
    interpolation: { escapeValue: false },
  });
  const t = i18n.getFixedT('en');

  assert.deepEqual(await getCodexAccountRefreshFeedback(async () => ({ ok: true }), t), {
    text: 'Refreshed', tone: 'success',
  });
  assert.equal(await getCodexAccountRefreshFeedback(async () => undefined, t), null);
  assert.deepEqual(
    await getCodexAccountRefreshFeedback(async () => ({ ok: false, error: new Error('Usage unavailable') }), t),
    { text: 'Refresh failed: Usage unavailable', tone: 'error' },
  );
  assert.deepEqual(
    await getCodexAccountRefreshFeedback(async () => { throw new Error('PROXY_RUNTIME_BUSY'); }, t),
    { text: 'Refresh failed: Proxy is busy', tone: 'error' },
  );
});

test('manual single/batch refresh clears stale messages and reports wrapped local failures without losing rejection', async () => {
  for (const raw of ['PROXY_RUNTIME_BUSY', 'Token 已过期，刷新 Token 失败: Token 已过期且刷新失败: PROXY_RUNTIME_BUSY']) {
    for (const error of [raw, new Error(raw)]) {
      const messages: unknown[] = [];
      let calls = 0;
      const refresh = async () => { calls += 1; throw error; };
      await assert.rejects(
        refreshCodexQuotaWithFeedback(refresh, key => `translated:${key}`, message => messages.push(message)),
        received => received === error,
      );
      assert.equal(calls, 1);
      assert.deepEqual(messages, [null, { text: 'translated:codex.proxy.runtimeBusy', tone: 'error' }]);
    }
  }
});

test('manual refresh retains official/unknown failures and never translates their body as local errors', async () => {
  for (const error of ['API 返回错误 403 [error_code:PROXY_RUNTIME_BUSY]', 'PROXY_UNKNOWN_FAILURE']) {
    const messages: unknown[] = [];
    await assert.rejects(
      refreshCodexQuotaWithFeedback(async () => { throw error; }, () => { throw new Error('must not translate'); }, message => messages.push(message)),
      received => received === error,
    );
    assert.deepEqual(messages, [null]);
  }
});

test('successful retry clears prior notice and does not add an error', async () => {
  const messages: unknown[] = [];
  await refreshCodexQuotaWithFeedback(async () => 3, key => key, message => messages.push(message));
  assert.deepEqual(messages, [null]);
});
