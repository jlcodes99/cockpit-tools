import assert from 'node:assert/strict';
import test from 'node:test';
import type { ProxyActivitySummaryEntry } from '../services/codexProxyActivityService';
import {
  PROXY_TRAFFIC_SAMPLE_LIMIT,
  applyProxyTrafficReading,
  appendProxyTrafficSample,
  computeProxyTrafficRates,
  createProxyTraffic,
  formatTrafficBytes,
  sumProxyActivitySummary,
  type ProxyTrafficReading,
} from './codexProxyTraffic';

const entry = (value: Partial<ProxyActivitySummaryEntry> & { accountId: string }): ProxyActivitySummaryEntry => ({
  supported: true,
  enabled: true,
  connectionCount: 0,
  upload: 0,
  download: 0,
  ...value,
});

const reading = (at: number, upload: number, download: number, activeConnections = 0): ProxyTrafficReading => ({
  at,
  totals: { upload, download, activeConnections, supported: true },
});

test('summary totals add every account and stay observable when one account is unsupported', () => {
  const totals = sumProxyActivitySummary([
    entry({ accountId: 'a', upload: 1024, download: 2048, connectionCount: 2 }),
    entry({ accountId: 'b', supported: false, upload: 0, download: 0 }),
    entry({ accountId: 'c', upload: 512, download: 256, connectionCount: 1, enabled: false }),
  ]);
  assert.deepEqual(totals, { upload: 1536, download: 2304, activeConnections: 3, supported: true });
  // No observable traffic at all is reported as unavailable, not as a hard failure.
  assert.equal(sumProxyActivitySummary([entry({ accountId: 'a', supported: false })]).supported, false);
  assert.equal(sumProxyActivitySummary([]).supported, true);
  // Broken payload values never poison the totals.
  const broken = sumProxyActivitySummary([entry({ accountId: 'a', upload: Number.NaN, download: -5, connectionCount: Number.NaN })]);
  assert.deepEqual(broken, { upload: 0, download: 0, activeConnections: 0, supported: true });
});

test('rates use the elapsed time and drop to zero when a counter moves backwards', () => {
  const previous = sumProxyActivitySummary([entry({ accountId: 'a', upload: 1000, download: 2000 })]);
  const next = sumProxyActivitySummary([entry({ accountId: 'a', upload: 4000, download: 2000 })]);
  assert.deepEqual(computeProxyTrafficRates(previous, next, 3000), { upPerSecond: 1000, downPerSecond: 0 });
  const restarted = sumProxyActivitySummary([entry({ accountId: 'a', upload: 10, download: 5 })]);
  const rates = computeProxyTrafficRates(previous, restarted, 3000);
  assert.deepEqual(rates, { upPerSecond: 0, downPerSecond: 0 });
  assert.ok(rates.upPerSecond >= 0 && Number.isFinite(rates.upPerSecond));
  assert.deepEqual(computeProxyTrafficRates(null, next, 3000), { upPerSecond: 0, downPerSecond: 0 });
  assert.deepEqual(computeProxyTrafficRates(previous, next, 0), { upPerSecond: 0, downPerSecond: 0 });
  assert.deepEqual(computeProxyTrafficRates(previous, next, Number.NaN), { upPerSecond: 0, downPerSecond: 0 });
});

test('samples are trimmed to the ring buffer limit', () => {
  const samples = Array.from({ length: PROXY_TRAFFIC_SAMPLE_LIMIT }, (_value, index) => ({ at: index, up: 0, down: 0 }));
  const next = appendProxyTrafficSample(samples, { at: 500, up: 1, down: 2 });
  assert.equal(next.length, PROXY_TRAFFIC_SAMPLE_LIMIT);
  assert.equal(next[0].at, 1);
  assert.deepEqual(next[next.length - 1], { at: 500, up: 1, down: 2 });
  assert.equal(appendProxyTrafficSample([], { at: 1, up: 0, down: 0 }, 0).length, 1);
});

test('session totals accumulate observed deltas since mount and keep counting across a failed poll', () => {
  const first = reading(1000, 1000, 2000, 2);
  const afterFirst = applyProxyTrafficReading(createProxyTraffic(), null, first);
  assert.equal(afterFirst.ready, true);
  assert.equal(afterFirst.sessionUpload, 0);
  assert.equal(afterFirst.sessionDownload, 0);
  assert.equal(afterFirst.activeConnections, 2);
  assert.deepEqual(afterFirst.samples, [{ at: 1000, up: 0, down: 0 }]);

  const second = reading(4000, 4000, 2600, 4);
  const afterSecond = applyProxyTrafficReading(afterFirst, first, second);
  assert.equal(afterSecond.sessionUpload, 3000);
  assert.equal(afterSecond.sessionDownload, 600);
  assert.deepEqual(afterSecond.samples[1], { at: 4000, up: 1000, down: 200 });
  assert.equal(afterSecond.activeConnections, 4);

  const failed = applyProxyTrafficReading(afterSecond, second, { at: 7000, totals: null });
  assert.equal(failed.ready, true);
  assert.equal(failed.unavailable, true);
  assert.equal(failed.upPerSecond, 0);
  assert.equal(failed.downPerSecond, 0);
  assert.equal(failed.sessionUpload, 3000);
  assert.equal(failed.sessionDownload, 600);
  assert.equal(failed.activeConnections, 4);
  assert.deepEqual(failed.samples, afterSecond.samples);

  const unsupported = applyProxyTrafficReading(failed, second, {
    at: 10000,
    totals: { upload: 0, download: 0, activeConnections: 0, supported: false },
  });
  assert.equal(unsupported.unavailable, true);
  assert.equal(unsupported.sessionUpload, 3000);
  assert.equal(unsupported.samples.length, 2);
});

test('byte formatting covers B, KB, MB and GB', () => {
  assert.equal(formatTrafficBytes(0), '0 B');
  assert.equal(formatTrafficBytes(999), '999 B');
  assert.equal(formatTrafficBytes(1024), '1.00 KB');
  assert.equal(formatTrafficBytes(1536), '1.50 KB');
  assert.equal(formatTrafficBytes(20 * 1024), '20.0 KB');
  assert.equal(formatTrafficBytes(1024 * 1024), '1.00 MB');
  assert.equal(formatTrafficBytes(150 * 1024 * 1024), '150 MB');
  assert.equal(formatTrafficBytes(1024 * 1024 * 1024), '1.00 GB');
  assert.equal(formatTrafficBytes(-1), '0 B');
  assert.equal(formatTrafficBytes(Number.NaN), '0 B');
});
