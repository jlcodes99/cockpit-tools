import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import type { ProxyCatalog } from '../../services/codexProxyCatalogService';
import type { CodexProxyWorkspaceValue } from './CodexProxyWorkspaceContext';

function catalog(revision: string): ProxyCatalog {
  return { sources: [{
    id: 'subscription', name: 'Example', kind: 'subscription', revision, updatedAt: 0, lastAttemptAt: null,
    autoUpdate: false, error: null, default: null, defaultInvalidated: false,
    nodes: [{ id: 'node', name: 'Node', protocol: 'hysteria2', supported: true, error: null }], groups: [],
  }] };
}

function setup() {
  const reads: ReturnType<typeof deferred<ProxyCatalog>>[] = [];
  const h = loadHookModule(new URL('./CodexProxyWorkspaceContext.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../services/codexProxyCatalogService': {
      getProxyCatalog: () => { const read = deferred<ProxyCatalog>(); reads.push(read); return read.promise; },
      catalogErrorKey: () => 'codex.proxy.catalog.errorRead',
    },
    '../../services/codexUnifiedProxyService': { getCodexUnifiedProxy: async () => null, unifiedProxyErrorKey: () => 'unified-error' },
    './useCodexProxyTraffic': { useCodexProxyTraffic: () => ({}) },
    '../../utils/codexAccountProxy': { canUseCodexAccountProxy: () => true },
  });
  const accounts: never[] = [];
  h.render(() => h.exports.CodexProxyWorkspaceProvider({ accounts, children: null }));
  const state = (): CodexProxyWorkspaceValue => h.flush().props.value;
  return { h, reads, state };
}

for (const oldOutcome of ['success', 'failure'] as const) {
  test(`a completed catalog write wins over an earlier background ${oldOutcome}; later reloads remain usable`, async () => {
    const { h, reads, state } = setup();
    assert.equal(reads.length, 1);
    assert.equal(state().catalogLoading, true);
    // Establish a visible read error, then start the older background retry.
    reads[0].reject(new Error('initial read failed'));
    await settlePromises();
    assert.equal(state().catalogError, 'codex.proxy.catalog.errorRead');
    assert.equal(state().catalogLoading, false);
    state().reloadCatalog(); state();
    assert.equal(reads.length, 2);
    assert.equal(state().catalogLoading, true);

    const written = catalog('permission-or-refresh-write');
    state().acceptCatalog(written);
    assert.equal(state().catalog, written);
    assert.equal(state().catalogError, '');
    assert.equal(state().catalogLoading, false);
    assert.equal(reads.length, 2, 'accepting a completed write must not start a replacement read');

    // Do not reload yet: the old effect is still live, so acceptCatalog must invalidate it.
    if (oldOutcome === 'success') reads[1].resolve(catalog('stale-background-read'));
    else reads[1].reject(new Error('late background failure'));
    await settlePromises();
    assert.equal(state().catalog, written, 'a stale read cannot undo the completed permission or refresh write');
    assert.equal(state().catalogError, '', 'a stale failure cannot reintroduce an error after a successful write');
    assert.equal(state().catalogLoading, false);

    state().reloadCatalog(); state();
    assert.equal(reads.length, 3);
    assert.equal(state().catalogLoading, true);
    assert.equal(state().catalog, written, 'a new read retains the usable catalog while loading');
    const latest = catalog('newer-reload');
    reads[2].resolve(latest);
    await settlePromises();
    assert.equal(state().catalog, latest, 'acceptCatalog must not permanently suppress future reloads');
    assert.equal(state().catalogError, '');
    assert.equal(state().catalogLoading, false);
    h.unmount();
  });
}
