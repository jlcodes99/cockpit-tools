import assert from 'node:assert/strict';
import test from 'node:test';
import { codexWakeupResultCode, codexWakeupResultLabel, latestCodexWakeupByAccount } from './useCodexAccountWakeupResults';
import type { CodexWakeupHistoryItem } from '../types/codexWakeup';

function result(accountId: string, timestamp: number, patch: Partial<CodexWakeupHistoryItem> = {}): CodexWakeupHistoryItem {
  return {
    id: `${accountId}-${timestamp}`,
    run_id: 'fixture',
    timestamp,
    trigger_type: 'scheduled',
    account_id: accountId,
    account_email: 'fixture@example.invalid',
    success: true,
    ...patch,
  };
}

test('selects the latest wakeup result independently for each account', () => {
  const older = result('account-a', 1);
  const failure = result('account-a', 4, { success: false });
  const other = result('account-b', 2);
  const results = latestCodexWakeupByAccount([failure, other, older]);
  assert.equal(results.get('account-a'), failure);
  assert.equal(results.get('account-b'), other);
  assert.equal(results.has('account-without-history'), false);
});

test('shows recorded status codes and omits unrecognized error text', () => {
  assert.equal(codexWakeupResultCode(result('a', 1, { http_status: 201 })), 'HTTP 201');
  assert.equal(codexWakeupResultCode(result('a', 1, { cli_path: 'fixture-codex' })), 'CLI 0');
  for (const [error, code] of [
    ['API 直连唤醒失败(429): private message', 'HTTP 429'],
    ['Codex CLI 退出失败: exit status: 2', 'CLI 2'],
    ['Codex CLI 退出失败: exit code: 0xc0000005', 'CLI 0xc0000005'],
    ['CHATGPT_PLAN_USAGE_LIMIT', 'CHATGPT_PLAN_USAGE_LIMIT'],
    ['Cannot connect: an arbitrary 403 inside free text', undefined],
    ['Codex 唤醒测试已取消', undefined],
  ] as const) {
    assert.equal(codexWakeupResultCode(result('a', 1, { success: false, error })), code);
  }
  assert.equal(codexWakeupResultCode(result('a', 1)), undefined);
});

test('labels failure and cancellation separately on the account card', () => {
  const translate = (key: string) => key.slice(key.lastIndexOf('.') + 1);
  assert.equal(codexWakeupResultLabel(result('a', 1, { success: false, error: 'Codex 唤醒测试已取消' }), translate), 'tab · cancelled');
  assert.equal(codexWakeupResultLabel(result('a', 1, { success: false, error: 'API 直连唤醒失败(503): example' }), translate), 'tab · historyFailed · HTTP 503');
});
