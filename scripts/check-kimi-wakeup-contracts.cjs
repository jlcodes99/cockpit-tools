/**
 * Lightweight contract checks for Kimi IPC mappers (no test runner required).
 * Run: node scripts/check-kimi-wakeup-contracts.cjs
 */

function assert(cond, msg) {
  if (!cond) {
    console.error('FAIL:', msg);
    process.exitCode = 1;
  } else {
    console.log('ok:', msg);
  }
}

// Mirror of toRawTask accountIds field (must not regress to account_ids).
function toRawTask(task) {
  return {
    id: task.id,
    name: task.name,
    enabled: task.enabled,
    accountIds: task.account_ids ?? [],
    prompt: task.prompt,
    model: task.model,
    schedule: {
      kind: task.schedule.kind,
      dailyTime: task.schedule.daily_time,
      weeklyDays: task.schedule.weekly_days ?? [],
      weeklyTime: task.schedule.weekly_time,
      intervalHours: task.schedule.interval_hours,
      quotaResetWindow: task.schedule.quota_reset_window,
      startupDelayMinutes: task.schedule.startup_delay_minutes,
    },
    createdAt: task.created_at,
    updatedAt: task.updated_at,
    lastRunAt: task.last_run_at,
  };
}

function toRawState(state) {
  return {
    enabled: state.enabled,
    tasks: (state.tasks ?? []).map(toRawTask),
  };
}

function fromRawKimiAccount(raw) {
  return {
    id: raw.id,
    email: raw.email,
    access_token: '',
    user_id: raw.userId,
    plan_type: raw.planType,
    created_at: raw.createdAt,
    last_used: raw.lastUsed,
    quota_query_last_error: raw.quotaQueryLastError,
    status_reason: raw.statusReason,
  };
}

const rawState = toRawState({
  enabled: true,
  tasks: [
    {
      id: 't1',
      name: 'wake',
      enabled: true,
      account_ids: ['acc-1', 'acc-2'],
      prompt: 'hi',
      model: 'kimi-code/k3',
      schedule: {
        kind: 'interval',
        interval_hours: 6,
        weekly_days: [],
      },
      created_at: 100,
      updated_at: 100,
      last_run_at: undefined,
    },
  ],
});

assert(Array.isArray(rawState.tasks[0].accountIds), 'toRawState emits accountIds array');
assert(
  rawState.tasks[0].accountIds.join(',') === 'acc-1,acc-2',
  'accountIds preserves order',
);
assert(
  !Object.prototype.hasOwnProperty.call(rawState.tasks[0], 'account_ids'),
  'wire payload must not use snake account_ids',
);

const mapped = fromRawKimiAccount({
  id: 'a1',
  email: 'x@y.z',
  accessToken: 'SECRET_SHOULD_STRIP',
  userId: 'u1',
  planType: 'plus',
  createdAt: 10,
  lastUsed: 20,
  quotaQueryLastError: null,
  statusReason: null,
});
assert(mapped.access_token === '', 'account mapper strips token');
assert(mapped.created_at === 10, 'createdAt → created_at');
assert(mapped.user_id === 'u1', 'userId → user_id');
assert(mapped.plan_type === 'plus', 'planType → plan_type');

if (process.exitCode) {
  console.error('contract checks failed');
  process.exit(1);
}
console.log('all kimi wakeup contract checks passed');
