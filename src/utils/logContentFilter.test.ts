import assert from 'node:assert/strict';
import test from 'node:test';
import { filterLogContent, startLogFilter, type LogFilterWorker } from './logContentFilter.ts';

const content = '2026-10-03 INFO Started\n2026-10-03 ERROR Failed\n  at TOKEN_REFRESH\n2026-10-03 WARN Retry';
test('search retains multiline entries and composes with the level filter', () => {
  assert.equal(filterLogContent({ content, level: 'ERROR', pattern: 'token_refresh', regex: false }).content,
    '2026-10-03 ERROR Failed\n  at TOKEN_REFRESH');
  assert.equal(filterLogContent({ content, level: 'INFO', pattern: 'token', regex: false }).content, '');
  assert.equal(filterLogContent({ content, level: 'ALL', pattern: 'Failed|Retry', regex: true }).content,
    '2026-10-03 ERROR Failed\n  at TOKEN_REFRESH\n2026-10-03 WARN Retry');
});
test('empty search is unchanged and invalid regex is reported', () => {
  assert.equal(filterLogContent({ content, level: 'ALL', pattern: '', regex: true }).content, content);
  assert.equal(filterLogContent({ content, level: 'ALL', pattern: '[', regex: true }).error, 'invalidRegex');
});
test('a stalled worker is terminated and cannot deliver a stale result', async () => {
  let terminated = 0;
  const worker: LogFilterWorker = { onmessage: null, onerror: null, postMessage() {}, terminate() { terminated++; } };
  const results: unknown[] = [];
  startLogFilter(worker, { content, level: 'ALL', pattern: '(a+)+$', regex: true }, (value) => results.push(value), 10);
  const late = worker.onmessage;
  await new Promise((resolve) => setTimeout(resolve, 30));
  assert.equal(terminated, 1);
  assert.deepEqual(results, [{ content: '', error: 'timeout' }]);
  late?.({ data: { content: 'stale' } } as MessageEvent);
  assert.equal(results.length, 1);
});
test('canceling a filter terminates it without updating the closed view', () => {
  let terminated = false;
  const worker: LogFilterWorker = { onmessage: null, onerror: null, postMessage() {}, terminate() { terminated = true; } };
  const cancel = startLogFilter(worker, { content, level: 'INFO', pattern: '', regex: false }, () => assert.fail('canceled'));
  cancel();
  assert.ok(terminated);
});
