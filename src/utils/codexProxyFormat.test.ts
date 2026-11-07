import assert from 'node:assert/strict';
import test from 'node:test';
import {
  PROXY_UNKNOWN,
  formatProxyBytes,
  formatProxyClock,
  formatProxyDateTime,
  formatProxyDuration,
  formatProxyRate,
  proxyTimeMs,
  proxyUsagePercent,
} from './codexProxyFormat';

test('bytes stay 1024-based and match the activity panel wording', () => {
  assert.equal(formatProxyBytes(0), '0 B');
  assert.equal(formatProxyBytes(999), '999 B');
  assert.equal(formatProxyBytes(1024), '1.0 KB');
  assert.equal(formatProxyBytes(1536), '1.5 KB');
  assert.equal(formatProxyBytes(1024 * 1024), '1.0 MB');
  assert.equal(formatProxyBytes(150 * 1024 * 1024), '150.0 MB');
  // 异常值不产生 `NaN B` 之类的伪数据。
  assert.equal(formatProxyBytes(-1), '0 B');
  assert.equal(formatProxyBytes(Number.NaN), '0 B');
  assert.equal(formatProxyBytes(Number.POSITIVE_INFINITY), '0 B');
});

test('rates reuse the byte units and add the per-second suffix', () => {
  assert.equal(formatProxyRate(0), '0 B/s');
  assert.equal(formatProxyRate(2048), '2.0 KB/s');
  assert.equal(formatProxyRate(Number.NaN), '0 B/s');
});

test('durations pick the compact unit pair and never go negative', () => {
  assert.equal(formatProxyDuration(0), '0s');
  assert.equal(formatProxyDuration(12_400), '12s');
  assert.equal(formatProxyDuration(65_000), '1m 05s');
  assert.equal(formatProxyDuration(3_600_000), '1h 00m');
  assert.equal(formatProxyDuration(90_000_000), '1d 01h');
  assert.equal(formatProxyDuration(-5_000), '0s');
  assert.equal(formatProxyDuration(Number.NaN), '0s');
});

test('clock and date-time render in local time and degrade to the placeholder', () => {
  // 本机构造 + 本机渲染，结果与运行环境时区无关。
  const local = new Date(2024, 0, 2, 3, 4, 5).getTime();
  assert.equal(formatProxyClock(local), '03:04:05');
  assert.equal(formatProxyDateTime(local), '2024-01-02 03:04');
  assert.equal(proxyTimeMs('2024-01-02T03:04:05Z'), Date.parse('2024-01-02T03:04:05Z'));
  assert.equal(proxyTimeMs(null), null);
  assert.equal(proxyTimeMs(''), null);
  assert.equal(proxyTimeMs('not-a-date'), null);
  assert.equal(proxyTimeMs(Number.NaN), null);
  assert.equal(formatProxyClock('not-a-date'), PROXY_UNKNOWN);
  assert.equal(formatProxyDateTime(undefined), PROXY_UNKNOWN);
});

test('usage percent is clamped to 0–100 and hidden when the total is unusable', () => {
  assert.equal(proxyUsagePercent(0, 100), 0);
  assert.equal(proxyUsagePercent(50, 100), 50);
  assert.equal(proxyUsagePercent(150, 100), 100);
  assert.equal(proxyUsagePercent(-10, 100), 0);
  assert.equal(proxyUsagePercent(1, 3), 33.3);
  // 没有总量时不能画出假进度。
  assert.equal(proxyUsagePercent(10, 0), 0);
  assert.equal(proxyUsagePercent(10, Number.NaN), 0);
  assert.equal(proxyUsagePercent(Number.NaN, 100), 0);
});
