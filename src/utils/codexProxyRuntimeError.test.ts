import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex.ts';
import { getCodexAccountQuotaError, isLocalCodexProxyError } from './codexProxyRuntimeError.ts';
import { isBlockingCodexQuotaError } from './codexQuotaError.ts';
import { isCodexOverviewAccountAbnormal } from './codexAccountOverview.ts';
import { proxyErrorKey } from '../services/codexAccountProxyService.ts';

function account(message: string): CodexAccount {
  return {
    id: 'cached-local-failure', email: 'test@example.com',
    tokens: { access_token: 'at-opaque', refresh_token: '', id_token: '' },
    created_at: 1, last_used: 1,
    quota_error: { message, timestamp: 1 },
  };
}

test('local proxy errors and real token-refresh wrappers do not mark accounts abnormal', () => {
  for (const code of ['PROXY_RUNTIME_LIMIT', 'PROXY_RUNTIME_BUSY', 'PROXY_RUNTIME_CAPACITY', 'PROXY_RUNTIME_READ_TIMEOUT', 'PROXY_ENGINE_START_FAILED', 'ENGINE_INSTALL_BUSY']) {
    for (const prefix of ['', 'Token 已过期且刷新失败: ', 'Token 已过期，刷新 Token 失败: Token 已过期且刷新失败: ']) {
      const value = account(prefix + code);
      assert.equal(isLocalCodexProxyError(value.quota_error?.message), true);
      assert.equal(getCodexAccountQuotaError(value.quota_error), undefined);
      assert.equal(isBlockingCodexQuotaError(value.quota_error), false);
      assert.equal(isCodexOverviewAccountAbnormal(value), false);
    }
  }
});

test('upstream 401/403/429 responses retain quota errors even when their body or code contains local names', () => {
  for (const status of [401, 403, 429]) {
    const error = {
      code: 'PROXY_RUNTIME_LIMIT',
      message: `API 返回错误 ${status} [error_code:PROXY_RUNTIME_LIMIT] [body:PROXY_RUNTIME_BUSY]`,
      timestamp: 1,
    };
    assert.equal(isLocalCodexProxyError(error.message), false);
    assert.equal(getCodexAccountQuotaError(error), error);
    assert.equal(isBlockingCodexQuotaError(error), true);
  }
});

test('unknown proxy faults and arbitrary wrappers remain visible', () => {
  for (const message of ['PROXY_TARGET_FAILED', 'PROXY_NEW_FAILURE', 'status=401: PROXY_RUNTIME_LIMIT', 'API 返回错误 403: Token 已过期且刷新失败: PROXY_RUNTIME_BUSY']) {
    const error = { message, timestamp: 1 };
    assert.equal(isLocalCodexProxyError(message), false);
    assert.equal(getCodexAccountQuotaError(error), error);
  }
  assert.equal(isCodexOverviewAccountAbnormal(account('API 返回错误 401 Unauthorized')), true);
});

test('local quota errors do not hide independent account reauthorization requirements', () => {
  const value = account('PROXY_RUNTIME_LIMIT');
  value.requires_reauth = true;
  value.reauth_reason = 'token_invalidated';
  assert.equal(isCodexOverviewAccountAbnormal(value), true);
});

test('runtime messages distinguish busy, active capacity and read failure', () => {
  assert.equal(proxyErrorKey('PROXY_RUNTIME_LIMIT'), 'codex.proxy.runtimeBusy');
  assert.equal(proxyErrorKey(new Error('PROXY_RUNTIME_BUSY')), 'codex.proxy.runtimeBusy');
  assert.equal(proxyErrorKey('PROXY_RUNTIME_CAPACITY'), 'codex.proxy.runtimeCapacity');
  assert.equal(proxyErrorKey('PROXY_RUNTIME_FAILED'), 'codex.proxy.runtimeReadFailed');
  assert.equal(proxyErrorKey('PROXY_RUNTIME_READ_TIMEOUT'), 'codex.proxy.runtimeReadFailed');
});
