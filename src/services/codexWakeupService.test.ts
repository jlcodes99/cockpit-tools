import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('Codex wakeup service round-trips a quota-reset delay and older schedules without one', async () => {
  const { exports } = loadHookModule(new URL('./codexWakeupService.ts', import.meta.url), {
    '@tauri-apps/api/core': {
      invoke: async (command: string, payload: any) => {
        assert.equal(command, 'codex_wakeup_save_state');
        assert.equal(payload.tasks[0].schedule.quotaResetDelayMinutes, 5);
        assert.equal(payload.tasks[1].schedule.quotaResetDelayMinutes, undefined);
        return payload;
      },
    },
  });
  const oldTask = {
    id: 'old', name: 'Old task', enabled: true, account_ids: ['fixture'],
    schedule: { kind: 'quota_reset', weekly_days: [], quota_reset_window: 'primary_window' },
    created_at: 1, updated_at: 1,
  };
  const delayed = { ...oldTask, id: 'delayed', schedule: { ...oldTask.schedule, quota_reset_delay_minutes: 5 } };
  const state = await exports.saveCodexWakeupState(true, [delayed, oldTask], []);
  assert.equal(state.tasks[0].schedule.quota_reset_delay_minutes, 5);
  assert.equal(state.tasks[1].schedule.quota_reset_delay_minutes, undefined);
});

test('Codex wakeup service preserves dynamic pool tasks and reports the executed roster', async () => {
  const calls: string[] = [];
  const { exports } = loadHookModule(new URL('./codexWakeupService.ts', import.meta.url), {
    '@tauri-apps/api/core': {
      invoke: async (command: string, payload: any) => {
        calls.push(command);
        if (command === 'codex_wakeup_get_pool_accounts') return ['second', 'first'];
        assert.equal(command, 'codex_wakeup_save_state');
        assert.equal(payload.tasks[0].useApiServicePool, true);
        assert.equal(payload.tasks[0].accountIds.length, 0);
        return payload;
      },
    },
  });
  const state = await exports.saveCodexWakeupState(true, [{
    id: 'pool', name: 'Pool', enabled: true, account_ids: [], use_api_service_pool: true,
    schedule: { kind: 'startup', weekly_days: [] }, created_at: 1, updated_at: 1,
  }], []);
  assert.equal(state.tasks[0].use_api_service_pool, true);
  assert.equal(state.tasks[0].account_ids.length, 0);
  assert.deepEqual(await exports.loadCodexWakeupPoolAccounts(), ['second', 'first']);
  const progress = exports.fromRawWakeupProgressPayload({
    runId: 'run', triggerType: 'startup', total: 2, running: true,
    phase: 'batch_started', accountIds: ['second', 'first'],
  });
  assert.deepEqual(progress.account_ids, ['second', 'first']);
  assert.deepEqual(calls, ['codex_wakeup_save_state', 'codex_wakeup_get_pool_accounts']);
});
