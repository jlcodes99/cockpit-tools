import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function sectionHarness(refresh: () => Promise<void>) {
  const events: string[] = [];
  const h = loadHookModule(new URL('./CodexProxyResourcesSection.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    './CodexProxyResources': { CodexProxyResources: () => null },
    './CodexProxyAssignDialog': { CodexProxyAssignDialog: () => null },
    './CodexProxyWorkspaceContext': { useCodexProxyWorkspace: () => ({
      reloadCatalog: () => { events.push('catalog'); },
      reloadUnified: () => { events.push('unified'); },
    }) },
    '../../utils/codexProxyRemoval': { refreshCodexProxyAccounts: async () => {
      events.push('accounts');
      await refresh();
    } },
  });
  const section = h.render(() => h.exports.CodexProxyResourcesSection());
  return { events, changed: section.props.children[0].props.onBindingsChanged as () => Promise<void> };
}

test('source deletion wiring waits for account snapshots before reloading workspace data', async () => {
  const refresh = deferred<void>();
  const h = sectionHarness(() => refresh.promise);
  let completed = false;
  const pending = h.changed().then(() => { completed = true; });
  await settlePromises();
  assert.equal(completed, false);
  assert.deepEqual(h.events, ['accounts']);
  refresh.resolve(); await pending;
  assert.deepEqual(h.events, ['accounts', 'catalog', 'unified']);
});

test('failed account refresh still reloads catalog and unified exit and rejects for inline retry', async () => {
  const failure = new Error('account read failed');
  const h = sectionHarness(async () => { throw failure; });
  await assert.rejects(h.changed(), (error) => error === failure);
  assert.deepEqual(h.events, ['accounts', 'catalog', 'unified']);
});
