import assert from 'node:assert/strict';
import test from 'node:test';
import { refreshCodexQuotaWithFeedback } from './codexQuotaRefreshFeedback.ts';

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
