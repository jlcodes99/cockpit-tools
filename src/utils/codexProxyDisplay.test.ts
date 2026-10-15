import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import type { CodexProxyRuntimeStatus } from '../services/codexAccountProxyService';
import { CODEX_PROXY_DISPLAY_EVENT, CODEX_PROXY_DISPLAY_KEY, codexProxyCardPresentation, getCodexProxyDisplay, persistCodexProxyDisplay } from './codexProxyDisplay';

const account = { id: 'account', egress_proxy: { protocol: 'resource', name: 'Auto group', selectedName: 'Configured leaf' } } as CodexAccount;
const idle: CodexProxyRuntimeStatus = { account: 'idle', desktop: 'idle', sidecar: 'idle', accountPort: null, desktopPort: null, proxySource: 'account' };

test('saved automatic selection is never presented as a running node or a latency measurement', () => {
  for (const status of [null, idle, { ...idle, accountNode: 'stale node', accountSelection: { name: 'stale node', delayMs: 5, checkedAt: 10 } }]) {
    const view = codexProxyCardPresentation(account, status);
    assert.equal(view.saved?.name, 'Auto group');
    assert.deepEqual(view.nodes, []);
  }
});

test('shared routing uses the effective node and keeps distinct desktop and API paths', () => {
  const view = codexProxyCardPresentation({ ...account, egress_proxy: null }, {
    ...idle, proxySource: 'unified', effectiveProxy: { protocol: 'resource', name: 'Shared group' },
    account: 'running', sidecar: 'running', desktop: 'running',
    accountNode: 'API leaf', sidecarNode: 'API leaf', desktopNode: 'Desktop leaf',
    accountSelection: { name: 'API leaf', delayMs: 123, checkedAt: 10 },
    desktopSelection: { name: 'retired leaf', delayMs: 456, checkedAt: 20 },
  });
  assert.equal(view.modeKey, 'codex.proxy.modeUnified');
  assert.equal(view.saved?.name, 'Shared group');
  assert.deepEqual(view.nodes.map((node) => node.name), ['API leaf', 'Desktop leaf']);
  assert.equal(view.nodes[0].selection?.delayMs, 123);
  assert.equal(view.nodes[1].selection, undefined, 'a measurement for another node must not leak');
});

test('explicit no-proxy mode hides saved and runtime nodes even before a status refresh', () => {
  const view = codexProxyCardPresentation({ ...account, egress_proxy_disabled: true }, {
    ...idle, account: 'running', accountNode: 'old',
  });
  assert.equal(view.modeKey, 'codex.proxy.modeDisabled');
  assert.equal(view.saved, null);
  assert.deepEqual(view.nodes, []);
});

test('merged channels retain the most recent measurement of their current node', () => {
  const view = codexProxyCardPresentation(account, { ...idle,
    account: 'running', sidecar: 'running', accountNode: 'leaf', sidecarNode: 'leaf',
    accountSelection: { name: 'leaf', delayMs: 200, checkedAt: 10 },
    sidecarSelection: { name: 'leaf', delayMs: 150, checkedAt: 20 },
  });
  assert.equal(view.nodes.length, 1);
  assert.equal(view.nodes[0].selection?.delayMs, 150);
});

test('desktop forwarding failure is preserved even when its kernel still runs', () => {
  const view = codexProxyCardPresentation(account, { ...idle, desktop: 'running', desktopNode: 'leaf', desktopEntry: {
    state: 'listening', port: 1234, requestCount: 1, lastRequestState: 'failed', lastError: 'upstream failed',
  } });
  assert.equal(view.nodes[0].name, 'leaf');
  assert.equal(view.rows.find((row) => row.kind === 'desktop')?.state, 'failed');
});

test('display preference defaults to compact, persists across reads and broadcasts to all entries', () => {
  let stored: string | null = null;
  const events: Event[] = [];
  const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => { assert.equal(key, CODEX_PROXY_DISPLAY_KEY); return stored; },
    setItem: (key: string, value: string) => { assert.equal(key, CODEX_PROXY_DISPLAY_KEY); stored = value; },
  } });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { dispatchEvent: (event: Event) => events.push(event) } });
  try {
    assert.equal(getCodexProxyDisplay(), 'compact');
    stored = 'invalid';
    assert.equal(getCodexProxyDisplay(), 'compact');
    for (const mode of ['summary', 'detailed', 'compact'] as const) {
      assert.equal(persistCodexProxyDisplay(mode), true);
      assert.equal(getCodexProxyDisplay(), mode);
    }
    assert.deepEqual(events.map((event) => event.type), Array(3).fill(CODEX_PROXY_DISPLAY_EVENT));
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
      getItem: () => { throw new Error('blocked'); },
      setItem: () => { throw new Error('blocked'); },
    } });
    assert.equal(getCodexProxyDisplay(), 'compact');
    assert.equal(persistCodexProxyDisplay('detailed'), false);
    assert.equal(events.length, 3, 'failed saves must not announce a successful change');
  } finally {
    if (storageDescriptor) Object.defineProperty(globalThis, 'localStorage', storageDescriptor); else Reflect.deleteProperty(globalThis, 'localStorage');
    if (windowDescriptor) Object.defineProperty(globalThis, 'window', windowDescriptor); else Reflect.deleteProperty(globalThis, 'window');
  }
});
