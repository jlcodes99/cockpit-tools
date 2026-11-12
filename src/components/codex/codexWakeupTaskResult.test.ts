import assert from 'node:assert/strict';
import test from 'node:test';
import { createInstance } from 'i18next';
import zhCN from '../../locales/zh-CN.json';
import type { CodexWakeupTask } from '../../types/codexWakeup';
import { getCodexWakeupTaskResult } from './codexWakeupTaskResult';

const task: CodexWakeupTask = {
  id: 'wakeup',
  name: 'Scheduled wakeup',
  enabled: true,
  account_ids: ['first', 'second'],
  schedule: { kind: 'daily', weekly_days: [] },
  created_at: 1,
  updated_at: 2,
  last_status: 'error',
};

async function translate() {
  const i18n = createInstance();
  await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: zhCN } } });
  return i18n.t.bind(i18n);
}

test('task-level failures show the reason instead of an account count, including legacy results', async () => {
  const t = await translate();
  for (const [message, summary] of [
    ['CODEX_CONFIRMATION_EXPIRED', '确认超时'],
    ['CODEX_CONFIRMATION_DECLINED', '已拒绝执行'],
    ['CODEX_CONFIRMATION_DECLINED_OR_EXPIRED', '已拒绝执行或确认超时'],
  ]) {
    assert.deepEqual(getCodexWakeupTaskResult({
      ...task,
      last_message: message,
      last_failure_count: message === 'CODEX_CONFIRMATION_DECLINED_OR_EXPIRED' ? 1 : undefined,
    }, t), { summary });
  }
});

test('a completed account batch shows its actual counts and the first recorded failure', async () => {
  const t = await translate();
  assert.deepEqual(getCodexWakeupTaskResult({
    ...task,
    last_message: 'Upstream rejected the model',
    last_duration_ms: 0,
    last_success_count: 1,
    last_failure_count: 1,
  }, t), {
    summary: '成功 1 个，失败 1 个',
    accountError: 'Upstream rejected the model',
  });
});
