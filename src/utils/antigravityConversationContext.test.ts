import assert from 'node:assert/strict';
import test from 'node:test';
import type { TurnUsageRecord } from '../types/antigravityTokenUsage';
import { buildAntigravityConversationContext } from './antigravityConversationContext';

function turn(
  stepIndex: number,
  inputTokens: number,
  cacheReadTokens = 0,
  cacheWriteTokens = 0,
  isExact = true,
): TurnUsageRecord {
  return {
    stepIndex,
    timestamp: null,
    modelId: 'test-model',
    modelDisplayName: 'Test model',
    apiProvider: 'Test provider',
    isExact,
    source: 'transcript_jsonl',
    status: 0,
    exact: {
      inputTokens: isExact ? inputTokens : 0,
      outputTokens: 500,
      thinkingTokens: 300,
      cacheReadTokens,
      cacheWriteTokens,
      totalTokens: inputTokens + cacheReadTokens + cacheWriteTokens + 800,
    },
    estimated: {
      inputTokens: isExact ? 0 : inputTokens,
      outputTokens: 300,
      totalTokens: isExact ? 300 : inputTokens + 300,
    },
    estimatedCostUsd: 0,
    cacheSavingsUsd: 0,
  };
}

test('attributes the largest request-context growth to input and cache, independently of output', () => {
  const result = buildAntigravityConversationContext([
    turn(1, 20, 100, 15),
    turn(3, 30, 200, 0),
    turn(5, 10, 0, 5),
  ], [4, 2]);

  assert.deepEqual(result.largestGrowth, {
    stepIndex: 3,
    change: { total: 95, input: 10, cacheRead: 100, cacheWrite: -15 },
  });
  assert.deepEqual(result.entries.map((entry) => entry.kind === 'turn'
    ? [entry.turn.stepIndex, entry.context.kind === 'reported' && entry.context.amount.total, entry.change?.total]
    : [entry.stepIndex, 'compaction']), [
    [1, 135, undefined], [2, 'compaction'], [3, 230, 95], [4, 'compaction'], [5, 15, -215],
  ]);
  assert.deepEqual(result.coverage, { reported: 3, estimated: 0, missing: 0 });
});

test('keeps estimates and missing prompt data out of reported changes', () => {
  const result = buildAntigravityConversationContext([
    turn(0, 10),
    turn(1, 0),
    turn(2, 1000),
    turn(3, 200, 0, 0, false),
    turn(4, 0, 0, 0, false),
    turn(5, 2000),
    turn(6, 2004),
  ]);

  assert.deepEqual(result.coverage, { reported: 4, estimated: 1, missing: 2 });
  assert.equal(result.largestGrowth?.change.total, 4);
  assert.deepEqual(result.entries.map((entry) => entry.kind === 'turn' && entry.context.kind),
    ['reported', 'missing', 'reported', 'estimated', 'missing', 'reported', 'reported']);
});

test('shows located compressions even without turn data and keeps unlocated events separate', () => {
  const result = buildAntigravityConversationContext([], [5, 0, 5], 3);
  assert.deepEqual(result.entries, [
    { kind: 'compaction', stepIndex: 0 },
    { kind: 'compaction', stepIndex: 5 },
  ]);
  assert.equal(result.unplacedCompactions, 1);
  assert.equal(result.largestGrowth, null);
});
