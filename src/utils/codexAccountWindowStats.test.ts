import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { CodexAccount } from '../types/codex';
import type { CodexLocalAccessAccountWindowQuery, CodexLocalAccessAccountWindowStats } from '../types/codexLocalAccess';
import { buildCodexAccountWindowStatsScope, createCodexAccountWindowStatsReader, type CodexAccountWindowStatsSnapshot } from './codexAccountWindowStats';

const clock = 1_800_000_000;
const account = (id = 'a', reset = clock + 100): CodexAccount => ({ id, quota: {
  hourly_percentage: 80, weekly_percentage: 90, hourly_reset_time: reset,
  weekly_reset_time: clock + 86400, hourly_window_minutes: 300, weekly_window_minutes: 10080,
} } as CodexAccount);
const scope = (id = 'a', reset = clock + 100) => buildCodexAccountWindowStatsScope([account(id, reset)], [id], clock)!;
const last = <T,>(items: T[]) => items[items.length - 1];
const row = (id = 'a', count = 1): CodexLocalAccessAccountWindowStats => ({ accountId: id, windowKey: 'primary',
  requestCount: count, inputTokens: count * 10, cachedTokens: 2, outputTokens: 5, totalTokens: count * 10 + 5, estimatedCostUsd: 0.1 });

test('statistics scope ignores account order, display data, tokens and percentages, but follows members and semantic windows', () => {
  const original = [account('a'), account('b')];
  const first = buildCodexAccountWindowStatsScope(original, ['b', 'a', 'a'], clock)!;
  const updated = original.map((value) => ({ ...value, email: 'renamed@example.com', token_generation: 999,
    quota: { ...value.quota!, hourly_percentage: 22, weekly_percentage: 33 } })).reverse();
  assert.equal(first.key, buildCodexAccountWindowStatsScope(updated, ['a', 'b'], clock + 30)!.key);
  assert.notEqual(first.key, buildCodexAccountWindowStatsScope(updated, ['b'], clock)!.key);
  assert.notEqual(scope().key, scope('a', clock + 101).key);
  assert.notEqual(scope().key, buildCodexAccountWindowStatsScope([{ ...account(), quota: { ...account().quota!, hourly_window_minutes: 60 } }], ['a'], clock)!.key);
  assert.notEqual(scope().key, buildCodexAccountWindowStatsScope([account()], ['a'], clock + 101)!.key);
  assert.equal(buildCodexAccountWindowStatsScope(original, ['missing'], clock), null);
});

test('bursts coalesce into the latest scope, and metadata-only snapshots never restart the query', async (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const queries: CodexLocalAccessAccountWindowQuery[][] = [];
  const snapshots: CodexAccountWindowStatsSnapshot[] = [];
  const reader = createCodexAccountWindowStatsReader({ read: async (input) => { queries.push(input); return [row('b')]; },
    now: () => clock * 1000, onChange: (value) => snapshots.push(value), minRefreshMs: 0 });
  reader.setScope(scope()); reader.setScope(scope('b')); reader.setScope({ ...scope('b') });
  for (let i = 0; i < 50; i++) reader.refresh();
  context.mock.timers.tick(250); await settlePromises();
  assert.equal(queries.length, 1);
  assert.equal(queries[0][0].accountId, 'b');
  assert.equal(last(snapshots)?.byAccountId.b.primary?.requestCount, 1);
  reader.setScope({ ...scope('b') }); context.mock.timers.tick(1000); await settlePromises();
  assert.equal(queries.length, 1);
  reader.dispose();
});

test('scope changes and storms never overlap native work, and old A cannot overwrite B or a later A', async (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const jobs: ReturnType<typeof deferred<CodexLocalAccessAccountWindowStats[]>>[] = [];
  const queries: CodexLocalAccessAccountWindowQuery[][] = [];
  const snapshots: CodexAccountWindowStatsSnapshot[] = [];
  const reader = createCodexAccountWindowStatsReader({ read: (input) => { queries.push(input); const job = deferred<CodexLocalAccessAccountWindowStats[]>(); jobs.push(job); return job.promise; },
    now: () => clock * 1000, minRefreshMs: 0, onChange: (value) => snapshots.push(value) });
  reader.setScope(scope()); context.mock.timers.tick(250);
  reader.setScope(scope('b')); reader.setScope(scope());
  for (let i = 0; i < 100; i++) reader.refresh();
  context.mock.timers.tick(10_000); assert.equal(jobs.length, 1);
  jobs[0].resolve([row('a', 999)]); await settlePromises();
  assert.equal(snapshots.some((value) => value.ready), false);
  context.mock.timers.tick(250); assert.equal(jobs.length, 2);
  jobs[1].resolve([row('a', 2), row('b', 99)]); await settlePromises();
  assert.equal(last(snapshots)?.byAccountId.a.primary?.requestCount, 2);
  assert.equal(last(snapshots)?.byAccountId.b, undefined);
  assert.equal(queries[1][0].accountId, 'a');
  reader.dispose();
});

test('same-scope background errors retain confirmed values; retries use a new request endpoint and disposal rejects late work', async (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  let now = clock * 1000;
  const jobs: ReturnType<typeof deferred<CodexLocalAccessAccountWindowStats[]>>[] = [];
  const queries: CodexLocalAccessAccountWindowQuery[][] = [];
  const snapshots: CodexAccountWindowStatsSnapshot[] = [];
  const errors: unknown[] = [];
  const reader = createCodexAccountWindowStatsReader({ read: (input) => { queries.push(input); const job = deferred<CodexLocalAccessAccountWindowStats[]>(); jobs.push(job); return job.promise; },
    now: () => now, minRefreshMs: 0, onChange: (value) => snapshots.push(value), onError: (error) => errors.push(error) });
  reader.setScope(scope()); context.mock.timers.tick(250); jobs[0].resolve([row()]); await settlePromises();
  const confirmed = last(snapshots);
  now += 30_000; reader.refresh(); context.mock.timers.tick(250); jobs[1].reject(new Error('busy')); await settlePromises();
  assert.equal(last(snapshots), confirmed); assert.equal(errors.length, 1);
  reader.refresh(); context.mock.timers.tick(250); assert.equal(queries[2][0].endAt, now);
  reader.dispose(); jobs[2].resolve([row('a', 999)]); await settlePromises();
  assert.equal(last(snapshots), confirmed);
  reader.refresh(); context.mock.timers.tick(10_000); assert.equal(jobs.length, 3);
});

test('continuous quota events are bounded to one refresh per five seconds', async (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  let now = clock * 1000; let calls = 0;
  const reader = createCodexAccountWindowStatsReader({ read: async () => { calls++; return [row()]; }, now: () => now, onChange() {} });
  reader.setScope(scope()); context.mock.timers.tick(250); await settlePromises(); assert.equal(calls, 1);
  for (let i = 0; i < 40; i++) reader.refresh();
  now += 4_999; context.mock.timers.tick(4_999); await settlePromises(); assert.equal(calls, 1);
  now++; context.mock.timers.tick(1); await settlePromises(); assert.equal(calls, 2);
  reader.dispose();
});
