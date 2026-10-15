import assert from 'node:assert/strict';
import test from 'node:test';
import { aggregateOutputSpeed, formatOutputSpeed, outputTokensPerSecond, outputSpeedUnavailableReason, type OutputSpeedSample } from './codexOutputSpeed';

const sample = (values: Partial<OutputSpeedSample> = {}): OutputSpeedSample => ({
  success: true, errorCategory: '', outputTokens: 1000, outputGenerationMs: 10000, ...values,
});

test('speed uses output usage and its observed window without adding cache or reasoning tokens', () => {
  assert.equal(outputTokensPerSecond(sample()), 100);
  assert.equal(outputTokensPerSecond({ ...sample(), cachedTokens: 100000, reasoningTokens: 9000 } as OutputSpeedSample), 100);
  assert.equal(outputTokensPerSecond(sample({ outputTokens: 100, outputGenerationMs: 100 })), 1000);
});

test('legacy, unsupported, cancelled and failed requests and invalid time boundaries stay unknown', () => {
  for (const value of [null, undefined, NaN, Infinity, -1, 0, 99, 100.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.equal(outputTokensPerSecond(sample({ outputGenerationMs: value })), null);
  }
  for (const value of [0, 99, -1, NaN, Infinity, 100.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.equal(outputTokensPerSecond(sample({ outputTokens: value })), null);
  }
  assert.equal(outputTokensPerSecond(sample({ success: false })), null);
  assert.equal(outputTokensPerSecond(sample({ errorCategory: 'client_canceled' })), null);
});

test('page aggregate weights eligible durations and does not average individual speeds or include missing usage', () => {
  assert.deepEqual(aggregateOutputSpeed([sample(), sample({ outputTokens: 200, outputGenerationMs: 500 }), sample({ success: false }), sample({ outputGenerationMs: null })]), {
    tokensPerSecond: 1200 / 10.5, samples: 2,
  });
  assert.deepEqual(aggregateOutputSpeed([]), { tokensPerSecond: null, samples: 0 });
});

test('formatting never presents unavailable data as zero or infinity', () => {
  for (const value of [null, 0, -1, NaN, Infinity]) assert.equal(formatOutputSpeed(value), null);
  assert.equal(formatOutputSpeed(43.49), '43');
  assert.equal(formatOutputSpeed(0.2), '0.2');
  assert.equal(formatOutputSpeed(0.00001), '< 0.1');
});

test('unavailable reasons use output usage rather than total tokens and do not guess missing timing causes', () => {
  assert.equal(outputSpeedUnavailableReason(sample()), null);
  assert.equal(outputSpeedUnavailableReason(sample({ errorCategory: 'client_canceled' })), 'canceled');
  assert.equal(outputSpeedUnavailableReason(sample({ success: false })), 'failed');
  assert.equal(outputSpeedUnavailableReason(sample({ errorCategory: 'stream_incomplete' })), 'failed');
  for (const tokens of [0, -1, NaN, Infinity, 100.5]) {
    assert.equal(outputSpeedUnavailableReason(sample({ outputTokens: tokens })), 'missingUsage');
  }
  assert.equal(outputSpeedUnavailableReason({ ...sample({ outputTokens: 58, outputGenerationMs: null }), totalTokens: 74259 } as OutputSpeedSample), 'shortOutput');
  assert.equal(outputSpeedUnavailableReason(sample({ outputTokens: 401, outputGenerationMs: null })), 'missingTiming');
  for (const ms of [undefined, -1, 0, NaN, Infinity, 100.5]) {
    assert.equal(outputSpeedUnavailableReason(sample({ outputGenerationMs: ms })), 'missingTiming');
  }
  assert.equal(outputSpeedUnavailableReason(sample({ outputGenerationMs: 99 })), 'shortWindow');
  assert.equal(outputSpeedUnavailableReason(sample({ outputTokens: 100, outputGenerationMs: 100 })), null);
});
