import assert from 'node:assert/strict';
import test from 'node:test';
import { setImmediate } from 'node:timers/promises';
import { appendRuntimeSnapshot, singleFlightRead, proxyRuntimeRows, proxyRuntimeChanges, proxyPreviewBinding, type ProxyRuntimeSnapshot } from './codexProxyPreview';
import type { CodexProxyRuntimeStatus } from '../services/codexAccountProxyService';

const initial: CodexProxyRuntimeStatus = { account: 'idle', desktop: 'idle', sidecar: 'idle', accountPort: null, desktopPort: null };
const listening = { state: 'listening' as const, port: 45101, requestCount: 0, lastRequestState: 'none' as const, lastError: null };
const desktopRow = (status: CodexProxyRuntimeStatus) => proxyRuntimeRows(status).find((row) => row.kind === 'desktop')!;

test('the listening launch entry is ready before its lazy engine starts, with separate ports', () => {
  const ready = desktopRow({ ...initial, desktopEntry: listening });
  assert.equal(ready.state, 'ready');
  assert.equal(ready.port, 45101);
  assert.equal(ready.kernelState, 'idle');
  assert.equal(ready.kernelPort, null);
  const active = desktopRow({ ...initial, desktop: 'running', desktopPort: 45102, desktopEntry: {
    ...listening, requestCount: 1, lastRequestState: 'forwarded',
  } });
  assert.equal(active.state, 'running');
  assert.equal(active.port, 45101, 'the launched client uses the entry, not the engine port');
  assert.equal(active.kernelPort, 45102);
  assert.equal(active.entry?.requestCount, 1);
});

test('listening never masks unavailable kernels, failed forwarding or a stopped entry', () => {
  for (const desktop of ['missing', 'stopped', 'starting', 'unbound'] as const) {
    assert.equal(desktopRow({ ...initial, desktop, desktopEntry: listening }).state, desktop);
  }
  for (const state of ['failed', 'stopped'] as const) {
    assert.equal(desktopRow({ ...initial, desktop: 'running', desktopEntry: { ...listening, state } }).state, state === 'failed' ? 'entry_failed' : state);
  }
  assert.equal(desktopRow({ ...initial, desktopEntry: { ...listening, requestCount: 1, lastRequestState: 'connecting' } }).state, 'starting');
  assert.equal(desktopRow({ ...initial, desktop: 'running', desktopEntry: { ...listening, requestCount: 1, lastRequestState: 'failed' } }).state, 'failed');
  assert.equal(desktopRow({ ...initial, desktop: 'missing', desktopEntry: { ...listening, lastRequestState: 'failed' } }).state, 'missing');
});

test('legacy responses retain the engine state and port instead of inventing an entry', () => {
  for (const desktopEntry of [undefined, null]) {
    const row = desktopRow({ ...initial, desktopEntry, desktopPort: 45102 });
    assert.equal(row.state, 'idle');
    assert.equal(row.port, 45102);
    assert.equal(row.entry, undefined);
  }
});

test('effective proxy source distinguishes unified, independent and unbound summaries', () => {
  const saved = { protocol: 'resource', name: 'Saved independent' };
  const effective = { protocol: 'resource', name: 'Effective unified' };
  assert.deepEqual(proxyPreviewBinding(null, null), { source: 'unknown', summary: null });
  assert.deepEqual(proxyPreviewBinding(saved, null), { source: 'account', summary: saved });
  assert.deepEqual(proxyPreviewBinding(saved, initial), { source: 'account', summary: saved });
  assert.deepEqual(proxyPreviewBinding(null, initial), { source: 'none', summary: null });
  assert.deepEqual(proxyPreviewBinding(saved, { ...initial, proxySource: 'unified', effectiveProxy: effective }), { source: 'unified', summary: effective });
  assert.deepEqual(proxyPreviewBinding(saved, { ...initial, proxySource: 'account', effectiveProxy: null }), { source: 'account', summary: null });
  assert.deepEqual(proxyPreviewBinding(saved, { ...initial, proxySource: 'none' }), { source: 'none', summary: null });
});

test('entry request changes are recorded and source-only changes cannot create empty history rows', () => {
  const start = { ...initial, desktopEntry: listening };
  const history = appendRuntimeSnapshot([], start, 1);
  for (const change of [{ requestCount: 1 }, { lastRequestState: 'connecting' as const }, { lastRequestState: 'failed' as const, lastError: 'PROXY_CONNECT_FAILED' }, { state: 'stopped' as const }, { port: 45103 }]) {
    const next = { ...start, desktopEntry: { ...listening, ...change } };
    assert.equal(appendRuntimeSnapshot(history, next, 2).length, 2);
    assert.deepEqual(proxyRuntimeChanges(next, start).map((row) => row.kind), ['desktop']);
  }
  assert.equal(appendRuntimeSnapshot(history, { ...start, proxySource: 'unified', effectiveProxy: { protocol: 'resource', name: 'New source' } }, 2), history);
});

test('shared account/API channel merges when state, port and node agree, including unbound', () => {
  const shared: CodexProxyRuntimeStatus = { ...initial, account: 'running', sidecar: 'running', accountPort: 59273, sidecarPort: 59273, accountNode: 'node', sidecarNode: 'node' };
  assert.deepEqual(proxyRuntimeRows(shared).map((row) => row.kind), ['combined', 'desktop']);
  for (const difference of [{ sidecarPort: 1234 }, { sidecar: 'stopped' as const }, { sidecarNode: 'other' }, { sidecar: undefined }]) {
    assert.equal(proxyRuntimeRows({ ...shared, ...difference }).length, 3);
  }
  // 未配置账号代理时账号与 API 服务通道内容完全一致，必须合并；只有通道不同才拆开展示。
  const unbound: CodexProxyRuntimeStatus = { account: 'unbound', desktop: 'unbound', sidecar: 'unbound', accountPort: null, desktopPort: null, sidecarPort: null, accountNode: null, desktopNode: null, sidecarNode: null };
  assert.deepEqual(proxyRuntimeRows(unbound).map((row) => row.kind), ['combined', 'desktop']);
  assert.deepEqual(proxyRuntimeChanges({ ...unbound, sidecar: 'running', sidecarPort: 1234 }, unbound).map((row) => row.kind), ['account', 'sidecar']);
  // 尚未读到状态时仍按三条通道展示，避免把「未知」当成「共用通道」。
  assert.equal(proxyRuntimeRows(null).length, 3);
  assert.equal(proxyRuntimeChanges(shared).length, 2);
  assert.deepEqual(proxyRuntimeChanges(shared, shared), []);
  assert.deepEqual(proxyRuntimeChanges({ ...shared, desktop: 'running' }, shared).map((row) => row.kind), ['desktop']);
  assert.deepEqual(proxyRuntimeChanges({ ...shared, sidecarPort: 1234 }, shared).map((row) => row.kind), ['account', 'sidecar']);
  assert.deepEqual(proxyRuntimeChanges(shared, { ...shared, sidecarPort: 1234 }).map((row) => row.kind), ['combined']);
});

test('runtime activity starts with an observation, deduplicates unchanged state and caps at 20', () => {
  let history = appendRuntimeSnapshot([], initial, 1);
  assert.equal(appendRuntimeSnapshot(history, { ...initial }, 2), history);
  for (let i = 2; i <= 30; i++) history = appendRuntimeSnapshot(history, { ...initial, accountNode: `node-${i}` }, i);
  assert.equal(history.length, 20);
  assert.equal(history[0].timestamp, 30);
  assert.equal(history[19].timestamp, 11);
});

test('all three runtime paths, ports and selected nodes participate in change detection', () => {
  const history: ProxyRuntimeSnapshot[] = [{ timestamp: 1, status: initial }];
  for (const changed of [
    { account: 'running' }, { desktop: 'starting' }, { sidecar: 'running' },
    { accountPort: 1234 }, { desktopPort: 2345 }, { sidecarPort: 3456 },
    { accountNode: 'A' }, { desktopNode: 'B' }, { sidecarNode: 'C' },
  ] as Partial<CodexProxyRuntimeStatus>[]) {
    assert.equal(appendRuntimeSnapshot(history, { ...initial, ...changed }, 2).length, 2);
  }
});

test('concurrent reads share one native operation and isolate accounts', async () => {
  let count = 0;
  let resolve!: (value: string) => void;
  const pending = new Promise<string>((done) => { resolve = done; });
  const fetch = singleFlightRead(async (key) => { count++; return key === 'a' ? pending : key; });
  const a = fetch('a'); const same = fetch('a');
  assert.equal(await fetch('b'), 'b');
  assert.equal(count, 2);
  resolve('done');
  assert.deepEqual(await Promise.all([a, same]), ['done', 'done']);
  assert.equal(await fetch('a'), 'done');
  assert.equal(count, 3);
});

test('timeouts release UI but do not duplicate stuck native work; late completion permits recovery', async () => {
  let count = 0;
  let resolve!: (value: number) => void;
  const pending = new Promise<number>((done) => { resolve = done; });
  const fetch = singleFlightRead(() => { count++; return count === 1 ? pending : Promise.resolve(2); }, 5);
  await assert.rejects(fetch('a'), /PROXY_PROBE_TIMEOUT/);
  await assert.rejects(fetch('a'), /PROXY_PROBE_TIMEOUT/);
  assert.equal(count, 1);
  resolve(1); await pending;
  // The wrapper adopts the native promise; allow its cleanup reaction to run.
  await setImmediate();
  assert.equal(await fetch('a'), 2);
});

test('failed native reads are cleared so the same account can retry', async () => {
  let count = 0;
  const fetch = singleFlightRead(async () => {
    if (++count === 1) throw new Error('unavailable');
    return 'recovered';
  });
  await assert.rejects(fetch('a'), /unavailable/);
  assert.equal(await fetch('a'), 'recovered');
});


test('current delay stays attached to its selected node and stale measurements cannot label a new node', () => {
  const status = { account: 'running', desktop: 'idle', sidecar: 'running', accountPort: 8001, desktopPort: null, sidecarPort: 8001,
    accountNode: 'US', sidecarNode: 'US', accountSelection: { name: 'US', delayMs: 216, checkedAt: 1234 } } as const;
  const rows = proxyRuntimeRows(status);
  assert.equal(rows[0].selection?.delayMs, 216);
  const changed = proxyRuntimeRows({ ...status, accountNode: 'JP', sidecarNode: 'JP' });
  assert.equal(changed[0].selection, undefined);
});
