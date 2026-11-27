import type { TurnUsageRecord } from '../types/antigravityTokenUsage';

interface ContextAmount {
  total: number;
  input: number;
  cacheRead: number;
  cacheWrite: number;
}

type ContextSample =
  | { kind: 'reported'; amount: ContextAmount }
  | { kind: 'estimated'; total: number }
  | { kind: 'missing' };

type ContextTimelineEntry =
  | { kind: 'compaction'; stepIndex: number }
  | { kind: 'turn'; turn: TurnUsageRecord; context: ContextSample; change: ContextAmount | null };

function readContextSample(turn: TurnUsageRecord): ContextSample {
  if (turn.isExact) {
    const { inputTokens: input, cacheReadTokens: cacheRead, cacheWriteTokens: cacheWrite } = turn.exact;
    const total = input + cacheRead + cacheWrite;
    if (total > 0) return { kind: 'reported', amount: { total, input, cacheRead, cacheWrite } };
  } else if (turn.estimated.inputTokens > 0) {
    return { kind: 'estimated', total: turn.estimated.inputTokens };
  }
  return { kind: 'missing' };
}

export function buildAntigravityConversationContext(
  turns: readonly TurnUsageRecord[],
  compactionSteps: readonly number[] = [],
  compactionCount = compactionSteps.length,
) {
  const steps = [...new Set(compactionSteps)].sort((a, b) => a - b);
  let nextCompaction = 0;
  const entries: ContextTimelineEntry[] = [];
  const coverage = { reported: 0, estimated: 0, missing: 0 };
  let previous: { stepIndex: number; amount: ContextAmount } | null = null;
  let largestGrowth: { stepIndex: number; change: ContextAmount } | null = null;

  // The detail endpoint provides one turn per step in ascending step order.
  for (const turn of turns) {
    while (nextCompaction < steps.length && steps[nextCompaction] <= turn.stepIndex) {
      entries.push({ kind: 'compaction', stepIndex: steps[nextCompaction++] });
    }

    const context = readContextSample(turn);
    coverage[context.kind] += 1;
    let change: ContextAmount | null = null;
    if (context.kind === 'reported') {
      const { amount } = context;
      if (previous && previous.stepIndex < turn.stepIndex) {
        change = {
          total: amount.total - previous.amount.total,
          input: amount.input - previous.amount.input,
          cacheRead: amount.cacheRead - previous.amount.cacheRead,
          cacheWrite: amount.cacheWrite - previous.amount.cacheWrite,
        };
        if (change.total > (largestGrowth?.change.total ?? 0)) {
          largestGrowth = { stepIndex: turn.stepIndex, change };
        }
      }
      previous = { stepIndex: turn.stepIndex, amount };
    } else {
      previous = null;
    }
    entries.push({ kind: 'turn', turn, context, change });
  }

  for (const stepIndex of steps.slice(nextCompaction)) {
    entries.push({ kind: 'compaction', stepIndex });
  }

  return {
    entries,
    coverage,
    largestGrowth,
    unplacedCompactions: Math.max(0, compactionCount - steps.length),
  };
}
