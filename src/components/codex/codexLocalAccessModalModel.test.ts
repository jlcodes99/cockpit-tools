import assert from 'node:assert/strict';
import test from 'node:test';
import type { TFunction } from 'i18next';
import type { CodexAccount } from '../../types/codex';
import type { CodexLocalAccessAccountHealth, CodexLocalAccessAccountPoolHealth, CodexLocalAccessUsageStats } from '../../types/codexLocalAccess';
import {
  buildLocalAccessSummaryStats,
  formatRequestResultDetail,
  formatUsdCost,
  normalizeAccessScope,
  normalizeCustomRoutingPriority,
  normalizeCustomRoutingWeight,
  resolveAccountUsagePriority,
  summarizeAccountPoolHealth,
} from './codexLocalAccessModalModel';

const account = (id: string, patch: Partial<CodexAccount> = {}): CodexAccount => ({
  id, email: id, auth_mode: 'oauth', tokens: { id_token: '', access_token: '' }, created_at: 0, last_used: 0, ...patch,
});
const health = (accountId: string, patch: Partial<CodexLocalAccessAccountHealth> = {}): CodexLocalAccessAccountHealth => ({
  accountId, email: accountId, available: true, consecutiveFailures: 0,
  lastSuccessAt: null, lastFailureAt: null, lastFailureStatus: null, lastFailureCategory: null, lastFailureMessage: null,
  imageGenerationStatus: 'unknown', imageGenerationCheckedAt: null, schedulerAvailable: true, schedulerReason: null,
  schedulerNextRetryAt: null, cooldowns: [], ...patch,
});
const pool = (accountStatuses: CodexLocalAccessAccountPoolHealth['accountStatuses']): CodexLocalAccessAccountPoolHealth => ({
  apiKeyId: 'key', apiKeyLabel: 'Client', provider: 'codex', model: 'test', requestKind: 'text',
  errorCode: 'unavailable', errorMessage: '', diagnosticAvailable: true, candidateAuths: 0, scopedAuths: 0,
  availableAuths: 0, unavailableAuths: 0, modelExcludedAuths: 0, quotaReservedAuths: 0, imagePolicyBlockedAuths: 0,
  accountStatuses, lastFailureAt: 1,
});
const translate = ((key: string, options: unknown) => typeof options === 'string' ? options : JSON.stringify({ key, ...options as object })) as TFunction;

test('routing drafts retain their bounds, explicit LAN scope and preferred-over-backup priority', () => {
  assert.deepEqual([-1, 0, 2.6, 999, NaN, Infinity].map(normalizeCustomRoutingPriority), [0, 0, 3, 100, 0, 0]);
  assert.deepEqual([-1, 0, 2.6, 999, NaN, Infinity].map(normalizeCustomRoutingWeight), [1, 1, 3, 100, 1, 1]);
  assert.equal(normalizeAccessScope('lan'), 'lan');
  assert.equal(normalizeAccessScope('unexpected'), 'localhost');
  assert.equal(resolveAccountUsagePriority(), 'normal');
  assert.equal(resolveAccountUsagePriority({ isPreferred: false, isBackup: true }), 'lowest');
  assert.equal(resolveAccountUsagePriority({ isPreferred: true, isBackup: true }), 'highest');
});

test('health summary preserves recovery, missing-account, cooldown, quota and authentication precedence', () => {
  const ids = ['recovered', 'missing', 'cooling', 'quota', 'auth', 'scheduler', 'hidden', 'pool-blocked', 'ready'];
  const accounts = ids.filter((id) => id !== 'missing').map((id) => account(id, id === 'quota' ? { quota_error: { message: '429 Too Many Requests', timestamp: 1 } } : {}));
  const state = {
    recoverySuppressedAccountIds: [' recovered '],
    accountHealth: [health('recovered', { schedulerAvailable: false }),
      health('cooling', { schedulerAvailable: false, cooldowns: [{ modelId: 'test', nextRetryAt: 60_000, remainingMs: 100, reason: 'cooldown' }] }),
      health('quota', { schedulerAvailable: false }),
      health('auth', { consecutiveFailures: 3, lastFailureCategory: 'auth_refresh_failed' }),
      health('scheduler', { schedulerAvailable: false }), health('hidden', { available: false })],
    accountPoolHealth: [pool([{ accountId: ' pool-blocked ', accountEmail: '', available: false, reasonCode: 'pool', reasonMessage: '' }])],
  };
  const before = JSON.stringify({ accounts, state });
  assert.deepEqual(summarizeAccountPoolHealth(accounts, ids, state), {
    total: 9, available: 2, abnormal: 3, cooldown: 1, missing: 1, authError: 2, quotaLimited: 1,
  });
  assert.equal(JSON.stringify({ accounts, state }), before, 'display aggregation must not mutate account or scheduler state');
});

test('a pool without member diagnostics retains its existing unavailable summary without inventing auth errors', () => {
  assert.deepEqual(summarizeAccountPoolHealth([account('a'), account('b')], ['a', 'b'], {
    accountHealth: [], accountPoolHealth: [pool([])], recoverySuppressedAccountIds: ['b'],
  }), { total: 2, available: 1, abnormal: 0, cooldown: 0, missing: 0, authError: 0, quotaLimited: 0 });
  assert.deepEqual(summarizeAccountPoolHealth([account('a')], ['a'], null), {
    total: 1, available: 1, abnormal: 0, cooldown: 0, missing: 0, authError: 0, quotaLimited: 0,
  });
});

test('request details keep cancellation and upstream failures out of the ordinary failure count', () => {
  const usage = { successCount: 10, failureCount: 12, clientCanceledCount: 3, upstreamResponseFailedCount: 4, streamIncompleteCount: 2 } as CodexLocalAccessUsageStats;
  const detail = JSON.parse(formatRequestResultDetail(translate, usage));
  assert.equal(detail.success, '10'); assert.equal(detail.failed, '3');
  assert.equal(detail.canceled, '3'); assert.equal(detail.upstreamFailed, '4'); assert.equal(detail.incomplete, '2');
  assert.equal(JSON.parse(formatRequestResultDetail(translate, { ...usage, failureCount: 1 })).failed, '0');
  assert.equal(JSON.parse(formatRequestResultDetail(translate, null)).success, '0');
});

test('summary cards retain their order, compact values and empty-state cost and latency formatting', () => {
  const totals = { requestCount: 2_000, inputTokens: 1_500, outputTokens: 500, totalTokens: 2_000,
    cachedTokens: 400, reasoningTokens: 100, estimatedCostUsd: 0.005 } as CodexLocalAccessUsageStats;
  const stats = buildLocalAccessSummaryStats(translate, totals, 1_250, 75);
  assert.deepEqual(stats.map(({ key, value }) => ({ key, value })), [
    { key: 'requests', value: '2K' }, { key: 'tokens', value: '2K' }, { key: 'specialTokens', value: '500' },
    { key: 'cost', value: '$0.005000' }, { key: 'latency', value: '1.25s' },
  ]);
  assert.equal(JSON.parse(stats.find((item) => item.key === 'latency')!.detail).rate, 75);
  const empty = buildLocalAccessSummaryStats(translate, undefined, 0, 0);
  assert.equal(empty.find((item) => item.key === 'cost')!.value, '$0.00');
  assert.equal(empty.find((item) => item.key === 'latency')!.value, '--');
  assert.equal(formatUsdCost(0.0000001), '<$0.000001');
});
