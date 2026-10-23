import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';
import type { CodexLocalAccessAccountStats, CodexLocalAccessUsageStats } from '../types/codexLocalAccess';
import { aggregateCodexServiceAccountStats } from './codexServiceAccountStats';
import { reconcileChatGptPlanDisplaySources } from './chatgptPlanOverview';

const source = { id: 'source', email: 'original@example.com' } as CodexAccount;
const grant = { id: 'grant', source_account_id: source.id, source_binding_valid: true } as ChatGptPlanAccount;
const stat = (accountId: string, requests: number, updatedAt = 1): CodexLocalAccessAccountStats => ({
  accountId, email: 'historical@example.com', updatedAt,
  usage: { requestCount: requests, successCount: requests - 1, failureCount: 1,
    inputTokens: requests * 10, outputTokens: requests * 5, totalTokens: requests * 15,
    estimatedCostUsd: requests / 10, totalLatencyMs: requests * 100,
    clientCanceledCount: 1, upstreamResponseFailedCount: 0, streamIncompleteCount: 0,
    cachedTokens: requests, reasoningTokens: requests * 2,
  } as CodexLocalAccessUsageStats,
});

test('all source and official history is merged once per endpoint, regardless of current membership', () => {
  const rows = [stat(source.id, 2), stat('chatgpt_plan_grant', 3), stat('unrelated', 7)];
  const snapshot = JSON.stringify(rows);
  const result = aggregateCodexServiceAccountStats(rows, [source], [grant]);
  assert.equal(result.get(source.id), result.get('chatgpt_plan_grant'));
  const merged = result.get(source.id)!;
  assert.equal(merged.accountId, source.id);
  assert.equal(merged.email, source.email);
  assert.equal(merged.usage.requestCount, 5);
  assert.equal(merged.usage.inputTokens, 50);
  assert.equal(merged.usage.totalTokens, 75);
  assert.equal(merged.usage.estimatedCostUsd, 0.5);
  assert.equal(merged.usage.totalLatencyMs, 500);
  assert.equal(merged.usage.clientCanceledCount, 2);
  assert.equal(merged.usage.cachedTokens, 5);
  assert.equal(merged.usage.reasoningTokens, 10);
  assert.equal(merged.usage.imageRequestCount, 0, 'older snapshots may omit newer fields');
  assert.equal(result.get('unrelated')!.usage.requestCount, 7);
  assert.equal(JSON.stringify(rows), snapshot, 'display aggregation never rewrites recorded identities or counters');
});

test('duplicate endpoint snapshots choose the newest value instead of double-counting', () => {
  const older = stat('chatgpt_plan_grant', 3, 1);
  const latest = stat('chatgpt_plan_grant', 8, 5);
  for (const rows of [[older, latest, latest], [latest, older, latest]]) {
    const merged = aggregateCodexServiceAccountStats([stat(source.id, 2), ...rows], [source], [grant]).get(source.id)!;
    assert.equal(merged.usage.requestCount, 10);
    assert.equal(merged.updatedAt, 5);
  }
});

test('invalid or ambiguous links and a missing original account never merge historical usage', () => {
  const rows = [stat(source.id, 2), stat('chatgpt_plan_grant', 3)];
  for (const grants of [[], [{ ...grant, source_binding_valid: false }],
    [{ ...grant, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }],
    [grant, { ...grant, id: 'duplicate' }], [grant, grant]]) {
    const result = aggregateCodexServiceAccountStats(rows, [source], grants);
    assert.equal(result.get(source.id)!.usage.requestCount, 2);
    assert.equal(result.get('chatgpt_plan_grant')!.usage.requestCount, 3);
  }
  assert.equal(aggregateCodexServiceAccountStats(rows, [], [grant]).get('chatgpt_plan_grant')!.accountId, 'chatgpt_plan_grant');
});

test('a verified pending display association preserves historical grouping without becoming valid auth', () => {
  const previous = reconcileChatGptPlanDisplaySources([grant]);
  const pending = { ...grant, source_binding_valid: false };
  const cache = reconcileChatGptPlanDisplaySources([pending], previous.confirmed);
  const result = aggregateCodexServiceAccountStats([stat(source.id, 2), stat('chatgpt_plan_grant', 3)], [source], [pending], cache.displaySourceIds);
  assert.equal(result.get(source.id)!.usage.requestCount, 5);
  assert.equal(pending.source_binding_valid, false);
});

test('persisted endpoint attribution keeps detached official history grouped and deduplicated', () => {
  const endpoint = { ...source, id: 'chatgpt_plan_grant', auth_mode: 'apikey', api_provider_id: 'cockpit_chatgpt_plan', api_service_source_account_id: source.id } as CodexAccount;
  const rows = [stat(source.id, 2), stat(endpoint.id, 4, 5), stat(endpoint.id, 1, 2)];
  const result = aggregateCodexServiceAccountStats(rows, [source, endpoint], []);
  assert.equal(result.get(source.id), result.get(endpoint.id));
  assert.equal(result.get(source.id)!.usage.requestCount, 6);
  assert.equal(result.get(source.id)!.email, source.email);
  const missing = aggregateCodexServiceAccountStats(rows, [endpoint], []);
  assert.equal(missing.get(endpoint.id)!.usage.requestCount, 4);
});
