import type { CodexLocalAccessUsageEvent } from '../types/codexLocalAccess';

export type OutputSpeedSample = Pick<CodexLocalAccessUsageEvent,
  'success' | 'errorCategory' | 'outputTokens' | 'outputGenerationMs'>;

export type OutputSpeedUnavailableReason = 'canceled' | 'failed' | 'missingUsage' | 'shortOutput' | 'missingTiming' | 'shortWindow';

/** Explain only what the recorded sample proves; missing timing has no inferred cause. */
export function outputSpeedUnavailableReason(event: OutputSpeedSample): OutputSpeedUnavailableReason | null {
  if (event.errorCategory === 'client_canceled') return 'canceled';
  if (!event.success || event.errorCategory) return 'failed';
  if (!Number.isSafeInteger(event.outputTokens) || event.outputTokens <= 0) return 'missingUsage';
  if (event.outputTokens < 100) return 'shortOutput';
  const ms = event.outputGenerationMs;
  if (typeof ms !== 'number' || !Number.isSafeInteger(ms) || ms <= 0) return 'missingTiming';
  if (ms < 100) return 'shortWindow';
  return null;
}

// Match the sidecar's eligibility rules. Legacy or unsupported measurements
// remain unknown; never substitute total latency, characters or total tokens.
export function outputSpeedSample(event: OutputSpeedSample): { tokens: number; ms: number } | null {
  const { outputTokens: tokens, outputGenerationMs: ms } = event;
  if (outputSpeedUnavailableReason(event) !== null || typeof ms !== 'number') return null;
  return { tokens, ms };
}

export function outputTokensPerSecond(event: OutputSpeedSample): number | null {
  const sample = outputSpeedSample(event);
  return sample ? sample.tokens / (sample.ms / 1000) : null;
}

/** Weighted speed of the explicitly supplied samples, never a full-history average. */
export function aggregateOutputSpeed(events: readonly OutputSpeedSample[]): { tokensPerSecond: number | null; samples: number } {
  let tokens = 0;
  let ms = 0;
  let samples = 0;
  for (const event of events) {
    const sample = outputSpeedSample(event);
    if (!sample) continue;
    tokens += sample.tokens;
    ms += sample.ms;
    samples++;
  }
  return { tokensPerSecond: Number.isSafeInteger(tokens) && Number.isSafeInteger(ms) && ms > 0
    ? tokens / (ms / 1000) : null, samples };
}

export function formatOutputSpeed(value: number | null): string | null {
  if (value == null || !Number.isFinite(value) || value <= 0) return null;
  if (value >= 1) return Math.round(value).toLocaleString();
  // Preserve a positive measurement without rounding a small rate down to zero.
  return value < 0.1 ? '< 0.1' : value.toFixed(1);
}
