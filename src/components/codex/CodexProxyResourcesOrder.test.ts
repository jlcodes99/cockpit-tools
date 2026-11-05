import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import type { ProxyCatalog, ProxyCatalogSource } from '../../services/codexProxyCatalogService';
import * as selectionUtils from '../../utils/codexProxySelection';
import * as sourceErrors from '../../utils/codexProxySourceError';
import type { ProxyEngineReadiness } from '../../utils/codexProxySetup';

type Element = { type: unknown; props: Record<string, any> };
function nodes(value: any): Element[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  return value && typeof value === 'object' && value.props ? [value, ...nodes(value.props.children)] : [];
}
const source = (id: string) => ({ id, name: id, kind: 'manual', nodes: [], groups: [], revision: 'rev', default: null } as unknown as ProxyCatalogSource);
function harness(initialSources = ['a', 'b', 'c'].map(source), initialReadiness: ProxyEngineReadiness = 'ready') {
  let data: ProxyCatalog = { sources: initialSources };
  let readiness = initialReadiness;
  let catalogReads = 0;
  const assignments: any[][] = [];
  let preflights = 0;
  let commit!: (ids: string[]) => void;
  const pending = deferred<ProxyCatalog>();
  const writes: string[][] = [];
  const h = loadHookModule(new URL('./CodexProxyResources.tsx', import.meta.url), {
    'react-dom': { createPortal: (value: unknown) => value },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    './useCodexProxyExitEditor': { useCodexProxyAccountName: () => (value: string) => value },
    './useProxyLatency': { useProxyLatency: () => ({ running: false, results: {}, cancel() {} }) },
    './useCodexProxyEngine': { useCodexProxyEngine: () => ({ readiness }) },
    '../../utils/codexProxySourceError': sourceErrors,
    './useProxyResourceOrder': { useProxyResourceOrder(sources: ProxyCatalogSource[], disabled: boolean, callback: typeof commit) {
      commit = callback; return { orderedSources: sources, draggingId: null, begin() {}, hover() {}, finish() {}, moveBy() {}, disabled };
    } },
    './CodexProxyPicker': { CodexProxyPicker: 'picker' },
    './CodexProxyResourceForm': { CodexProxyResourceForm: 'form' },
    './CodexProxyStrategyPanel': { StrategyEditorDialog: 'editor', StrategyDeleteDialog: 'delete' },
    './CodexProxyResourceNodes': { CodexProxyResourceNodes: 'nodes' },
    './CodexProxyLatencyBadge': { CodexProxyLatencyBadge: 'latency' },
    './CodexProxySetupGuide': { CodexProxySetupGuide: 'guide' },
    '../../services/codexProxyEngineService': { preflightCodexProxyEngine() { preflights++; } },
    '../../services/codexProxyStrategyService': { strategyCandidates: () => [], strategyKindOf: () => '', strategyKindKey: () => '' },
    '../../utils/codexProxySelection': selectionUtils,
    '../../utils/codexProxyFormat': { formatProxyBytes: String, formatProxyDateTime: String },
    '../../utils/codexProxyRemoval': { proxyRemovalErrorKey: (error: unknown, fallback: (e: unknown) => string) => fallback(error) },
    '../../services/codexProxyCatalogService': {
      getProxyCatalog: async () => { catalogReads++; return data; },
      reorderProxyCatalog(ids: string[]) { writes.push(Array.from(ids)); return pending.promise; },
      catalogErrorKey: () => 'codex.proxy.catalog.changed',
    },
  });
  h.render(() => h.exports.CodexProxyResources({ onAssign(...args: any[]) { assignments.push(args); }, async onBindingsChanged() {} }));
  const rows = () => nodes(h.flush()).filter((node) => node.type === 'article').map((node) => nodes(node).find((child) => child.type === 'strong')!.props.children);
  return { h, pending, writes, rows, assignments, preflights: () => preflights, catalogReads: () => catalogReads,
    readiness(next: ProxyEngineReadiness) { readiness = next; h.flush(); },
    commit(ids: string[]) { h.flush(); commit(ids); }, data(next: ProxyCatalog) { data = next; } };
}

function warningRows(h: ReturnType<typeof harness>) {
  return nodes(h.h.flush()).filter((node) => node.type === 'article'
    && nodes(node).some((child) => child.props.className === 'codex-resource-entry-warning'))
    .map((node) => nodes(node).find((child) => child.type === 'strong')!.props.children);
}

test('confirmed engine recovery immediately supersedes old install errors without refetching or mutating sources', async () => {
  const entries = ['PROXY_ENGINE_MISSING', 'PROXY_ENGINE_VERSION', 'ENGINE_INSTALL_BUSY',
    'CATALOG_DOWNLOAD', 'PROXY_PROFILE_RESOURCE', 'PROXY_ENGINE_START_FAILED', 'PROXY_ENGINE_TIMEOUT',
    'ENGINE_INSTALL_VERIFY'].map((error) => ({ ...source(error), error }));
  entries.push({ ...source('invalid-default'), error: 'PROXY_ENGINE_MISSING', defaultInvalidated: true } as typeof entries[number]);
  const original = JSON.stringify(entries);
  const h = harness(entries, 'loading'); await settlePromises();
  for (const state of ['loading', 'unknown', 'missing', 'installing', 'failed', 'unsupported'] as const) {
    h.readiness(state); assert.equal(warningRows(h).length, entries.length);
  }
  h.readiness('ready');
  assert.deepEqual(warningRows(h), entries.slice(3).map((entry) => entry.name));
  assert.equal(h.catalogReads(), 1);
  assert.equal(h.preflights(), 0);
  assert.equal(JSON.stringify(entries), original);
  assert.deepEqual(h.writes, []);
  h.h.unmount();
});

test('reopening resources after installation does not revive a persisted missing-engine warning', async () => {
  const h = harness([{ ...source('old-error'), error: 'PROXY_ENGINE_MISSING' }], 'ready');
  await settlePromises();
  assert.deepEqual(warningRows(h), []);
  assert.deepEqual(h.rows(), ['old-error']);
  h.h.unmount();
});

test('resource reorder previews optimistically, blocks duplicate saves and adopts the persisted list', async () => {
  const h = harness(); await settlePromises(); assert.deepEqual(h.rows(), ['a', 'b', 'c']);
  h.commit(['c', 'a', 'b']); h.commit(['b', 'a', 'c']);
  assert.deepEqual(h.rows(), ['c', 'a', 'b']); assert.deepEqual(h.writes, [['c', 'a', 'b']]);
  const persisted = { sources: ['c', 'a', 'b'].map(source) }; h.data(persisted); h.pending.resolve(persisted); await settlePromises();
  assert.deepEqual(h.rows(), ['c', 'a', 'b']); h.h.unmount();
});

test('failed saving restores the durable order, reports the error and refreshes concurrent additions', async () => {
  const h = harness(); await settlePromises(); h.commit(['b', 'a', 'c']); assert.deepEqual(h.rows(), ['b', 'a', 'c']);
  h.data({ sources: ['a', 'b', 'c', 'new'].map(source) }); h.pending.reject(new Error('CATALOG_CHANGED')); await settlePromises();
  assert.deepEqual(h.rows(), ['a', 'b', 'c', 'new']);
  const alerts = nodes(h.h.flush()).filter((node) => node.props.role === 'alert');
  assert.equal(alerts.length, 1); assert.ok(alerts[0].props.children.includes('codex.proxy.catalog.changed')); h.h.unmount();
});

test('assignment opens immediately for subscriptions, strategies and single nodes without expanding or engine checks', async () => {
  const nodesForSource = ['one', 'two'].map((id) => ({ id, name: id, protocol: 'http', supported: true, error: null }));
  const subscription = { ...source('subscription'), kind: 'subscription' as const, nodes: nodesForSource };
  const strategy = { ...source('strategy'), kind: 'strategy' as const, nodes: nodesForSource,
    groups: [{ id: 'auto', name: 'auto', kind: 'url-test', members: ['one', 'two'], supported: true, error: null }] };
  const manual = { ...source('manual'), nodes: [nodesForSource[0]] };
  const saved = { ...subscription, id: 'saved', default: { itemId: 'two', groupId: null, selections: {} } };
  const h = harness([subscription, strategy, manual, saved]); await settlePromises();
  for (const row of nodes(h.h.flush()).filter((node) => node.type === 'article')) {
    const assign = nodes(row).find((node) => node.type === 'button' && Array.isArray(node.props.children) && node.props.children.includes('codex.proxy.managerResources.assign'))!;
    assign.props.onClick();
  }
  assert.deepEqual(h.assignments.map((args) => [args[0], args[1], args[3]]), [
    ['subscription', '', ''], ['strategy', 'auto', 'auto'], ['manual', 'one', ''], ['saved', 'two', ''],
  ]);
  assert.equal(h.preflights(), 0);
  assert.equal(nodes(h.h.flush()).some((node) => node.props.className === 'codex-resource-expanded'), false);
  h.h.unmount();
});

test('an incomplete explicit group is passed to the dialog instead of falling back to the saved default', async () => {
  const entry = { ...source('subscription'), kind: 'subscription' as const,
    nodes: [{ id: 'one', name: 'one', protocol: 'http', supported: true, error: null }],
    groups: [{ id: 'select', name: 'select', kind: 'select', members: ['one'], supported: true, error: null }],
    default: { itemId: 'one', groupId: null, selections: {} } };
  const h = harness([entry]); await settlePromises();
  nodes(h.h.flush()).find((node) => node.props.className === 'codex-resource-entry-main')!.props.onClick();
  nodes(h.h.flush()).find((node) => node.type === 'picker')!.props.choose('select', 'select');
  nodes(h.h.flush()).find((node) => node.type === 'button' && Array.isArray(node.props.children) && node.props.children.includes('codex.proxy.managerResources.assign'))!.props.onClick();
  assert.equal(h.assignments[0][1], 'select');
  assert.equal(h.assignments[0][3], 'select');
  assert.deepEqual(Object.keys(h.assignments[0][2]), []);
  h.h.unmount();
});


test('subscription resources expose automatic updates as information without an opt-out or certificate toggle', async () => {
  const entry = { ...source('subscription'), kind: 'subscription' as const, autoUpdate: false,
    nodes: [{ id: 'one', name: 'one', protocol: 'http', supported: true, insecure: true, error: null }] };
  const h = harness([entry]); await settlePromises();
  nodes(h.h.flush()).find((node) => node.props.className === 'codex-resource-entry-main')!.props.onClick();
  const tree = nodes(h.h.flush());
  assert.equal(tree.some((node) => node.props.role === 'switch'), false);
  assert.ok(tree.some((node) => node.type === 'p' && node.props.children === 'codex.proxy.catalog.autoUpdate'));
  assert.equal(tree.find((node) => node.type === 'nodes')!.props.onInsecure, undefined);
  h.h.unmount();
});
