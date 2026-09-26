import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import type { ProxyCatalog, ProxyCatalogGroup, ProxyCatalogSource } from '../services/codexProxyCatalogService';
import type { CodexUnifiedProxyView } from '../services/codexUnifiedProxyService';
import {
  batchUnbindTargets, codexProxyAccountName, exitDraftState, filterProxyAccounts, hasExitSelection,
  resolveExitMode, restoreExitChoice, sameExitChoice, summarizeProxyBatch, unifiedFollowingIds, unifiedProxyActive,
  type CodexProxyCatalogState, type CodexProxyExitChoice,
} from './codexProxyDraft';
import type { BatchBindResult } from './codexProxyBatch';

type Binding = CodexAccount['egress_proxy'];

const group = (id: string, kind: string, members: string[]): ProxyCatalogGroup => ({ id, name: id, kind, members, supported: true, error: null });
const source: ProxyCatalogSource = {
  id: 'subscription', name: 'Example', kind: 'subscription', revision: '1', updatedAt: 0, lastAttemptAt: null, autoUpdate: false, error: null,
  default: null, defaultInvalidated: false,
  nodes: [
    { id: 'node-a', name: 'Alpha', protocol: 'http', supported: true, error: null },
    { id: 'node-b', name: 'Beta', protocol: 'vless', supported: true, error: null },
    { id: 'bad', name: 'Unavailable', protocol: 'vless', supported: false, error: 'PROXY_TLS_INSECURE' },
  ],
  groups: [group('manual', 'select', ['Unavailable', 'Beta', 'Alpha']), group('auto', 'url-test', ['Alpha', 'Beta'])],
};
const catalog: ProxyCatalog = { sources: [source] };
const emptyCatalog: ProxyCatalog = { sources: [] };
const readyState: CodexProxyCatalogState = { catalog, loading: false, failed: false };

const account = (id: string, extra: Partial<CodexAccount> = {}): CodexAccount => ({ id, ...extra } as CodexAccount);
const binding = (sourceId: string, itemId: string, extra: Partial<NonNullable<Binding>> = {}): Binding =>
  ({ protocol: 'catalog', sourceId, itemId, ...extra }) as Binding;
const choice = (itemId: string, selections: Record<string, string> = {}, groupId?: string): CodexProxyExitChoice =>
  ({ sourceId: source.id, itemId, groupId: groupId ?? (source.groups.some((entry) => entry.id === itemId) ? itemId : ''), selections });

test('a saved binding restores its identity and explicit manual member', () => {
  const restored = restoreExitChoice(catalog, binding(source.id, 'manual', { selectedName: 'Beta' }));
  assert.deepEqual(restored, { sourceId: source.id, itemId: 'manual', groupId: 'manual', selections: { manual: 'Beta' } });
  // A deleted source must never silently select a replacement.
  assert.deepEqual(restoreExitChoice(catalog, binding('deleted', 'node-a')), { sourceId: '', itemId: '', groupId: '', selections: {} });
  // A deleted node keeps the source so the user can see what changed.
  assert.deepEqual(restoreExitChoice(catalog, binding(source.id, 'deleted')), { sourceId: source.id, itemId: '', groupId: '', selections: {} });
  // An unbound account only pre-selects the first usable source for browsing; no node is chosen.
  assert.deepEqual(restoreExitChoice(catalog, null), { sourceId: source.id, itemId: '', groupId: '', selections: {} });
  assert.deepEqual(restoreExitChoice(emptyCatalog, null), { sourceId: '', itemId: '', groupId: '', selections: {} });
});

test('a draft equals the saved binding only with the same identity and manual choices', () => {
  const bound = choice('manual', { manual: 'Beta' });
  assert.equal(sameExitChoice(bound, choice('manual', { manual: 'Beta' })), true);
  assert.equal(sameExitChoice(bound, choice('manual', { manual: 'Alpha' })), false);
  assert.equal(sameExitChoice(bound, choice('node-a')), false);
  assert.equal(sameExitChoice(choice('manual', { manual: 'Beta', nested: 'Alpha' }), bound), false);
  assert.equal(sameExitChoice(choice('auto', {}), choice('auto', {})), true);
});

test('dirty tracking distinguishes browsing state from a pending write', () => {
  const saved = binding(source.id, 'node-a');
  const restored = restoreExitChoice(catalog, saved);
  assert.deepEqual(exitDraftState(saved, restored, restored), { bound: true, saved: true, dirty: false });
  assert.deepEqual(exitDraftState(saved, restored, choice('node-b')), { bound: true, saved: false, dirty: true });
  // Reselecting the same node again is not a pending write.
  assert.deepEqual(exitDraftState(saved, restored, choice('node-a')), { bound: true, saved: true, dirty: false });
  // An unbound account only counts as dirty once a node or group is chosen.
  const unboundRestored: CodexProxyExitChoice = { sourceId: source.id, itemId: '', groupId: '', selections: {} };
  assert.deepEqual(exitDraftState(null, unboundRestored, unboundRestored), { bound: false, saved: false, dirty: false });
  assert.deepEqual(exitDraftState(null, unboundRestored, choice('node-a')), { bound: false, saved: false, dirty: true });
  assert.equal(hasExitSelection(unboundRestored), false);
});

test('the effective exit resolves independent, unified, default and stale', () => {
  assert.equal(resolveExitMode(binding(source.id, 'node-a'), readyState, true), 'independent');
  assert.equal(resolveExitMode(binding(source.id, 'auto'), readyState, false), 'independent');
  assert.equal(resolveExitMode(binding(source.id, 'gone'), readyState, false), 'stale');
  assert.equal(resolveExitMode(binding('deleted', 'node-a'), readyState, false), 'stale');
  // A direct binding cannot go stale, and an unloaded catalog must not report stale.
  assert.equal(resolveExitMode({ protocol: 'http', server: '127.0.0.1', port: 8080 } as Binding, readyState, false), 'independent');
  assert.equal(resolveExitMode(binding(source.id, 'gone'), { catalog: emptyCatalog, loading: true, failed: false }, false), 'independent');
  assert.equal(resolveExitMode(binding(source.id, 'gone'), { catalog: emptyCatalog, loading: false, failed: true }, false), 'independent');
  assert.equal(resolveExitMode(null, readyState, true), 'unified');
  assert.equal(resolveExitMode(null, readyState, false), 'default');
});

test('only unbound accounts follow an applied unified exit', () => {
  const accounts = [account('bound'), account('free')];
  const saved = (entry: CodexAccount): Binding => entry.id === 'bound' ? binding(source.id, 'node-a') : null;
  const unified = { mode: 'all_accounts', binding: { sourceId: source.id, itemId: 'node-a' } } as CodexUnifiedProxyView;
  assert.deepEqual(unifiedFollowingIds(accounts, saved, unified), ['free']);
  assert.deepEqual(unifiedFollowingIds(accounts, saved, { ...unified, mode: 'off', binding: null }), []);
  assert.deepEqual(unifiedFollowingIds(accounts, saved, null), []);
  assert.equal(unifiedProxyActive(unified), true);
  assert.equal(unifiedProxyActive({ ...unified, binding: null }), false);
});

test('search and binding-status filters compose over the account list', () => {
  const accounts = [
    account('one', { account_name: 'Work', email: 'work@example.com' }),
    account('two', { email: 'personal@example.com' }),
    account('three'),
  ];
  const saved = (entry: CodexAccount): Binding => entry.id === 'one' ? binding(source.id, 'node-a') : null;
  assert.deepEqual(filterProxyAccounts(accounts, { saved }).map((entry) => entry.id), ['one', 'two', 'three']);
  assert.deepEqual(filterProxyAccounts(accounts, { search: 'WORK', saved }).map((entry) => entry.id), ['one']);
  assert.deepEqual(filterProxyAccounts(accounts, { search: 'personal@', saved }).map((entry) => entry.id), ['two']);
  assert.deepEqual(filterProxyAccounts(accounts, { search: 'three', saved }).map((entry) => entry.id), ['three']);
  assert.deepEqual(filterProxyAccounts(accounts, { filter: 'bound', saved }).map((entry) => entry.id), ['one']);
  assert.deepEqual(filterProxyAccounts(accounts, { filter: 'unbound', saved }).map((entry) => entry.id), ['two', 'three']);
  assert.deepEqual(filterProxyAccounts(accounts, { search: 'nobody', filter: 'bound', saved }), []);
  assert.equal(codexProxyAccountName(account('three')), 'three');
  assert.equal(codexProxyAccountName(account('two', { email: 'personal@example.com' })), 'personal@example.com');
});

test('batch unbinding targets bound accounts and keeps per-account failures locatable', () => {
  const accounts = [account('bound', { account_name: 'Work' }), account('free')];
  const saved = (entry: CodexAccount): Binding => entry.id === 'bound' ? binding(source.id, 'node-a') : null;
  const targets = batchUnbindTargets(accounts, saved);
  assert.deepEqual(targets.map((entry) => entry.account.id), ['bound']);
  assert.deepEqual(targets.map((entry) => entry.willOverwrite), [true]);
  const cleared = accounts.map((entry) => ({ ...entry, egress_proxy: null }));
  assert.deepEqual(batchUnbindTargets(cleared, (entry) => entry.egress_proxy ?? null), []);
  assert.deepEqual(batchUnbindTargets(accounts, (entry) => entry.egress_proxy ?? null), []);

  const results: BatchBindResult[] = [
    { accountId: 'bound', ok: true },
    { accountId: 'free', ok: false, errorKey: 'codex.proxy.saveFailed' },
    { accountId: 'unknown', ok: false },
  ];
  const summary = summarizeProxyBatch(results, accounts);
  assert.deepEqual({ total: summary.total, success: summary.success, failed: summary.failed }, { total: 3, success: 1, failed: 2 });
  assert.deepEqual(summary.failures, [
    { accountId: 'free', name: 'free', errorKey: 'codex.proxy.saveFailed' },
    { accountId: 'unknown', name: 'unknown', errorKey: 'codex.proxy.saveFailed' },
  ]);
});
