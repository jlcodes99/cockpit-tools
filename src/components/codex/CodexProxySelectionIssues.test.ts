import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import * as catalogService from '../../services/codexProxyCatalogService';

function elements(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}

function harness() {
  const calls: { action: string; args: any[]; result: ReturnType<typeof deferred<any>> }[] = [];
  const applied: any[] = []; const cancelled: string[] = []; const pending: boolean[] = [];
  const invoke = (action: string) => (...args: any[]) => {
    const result = deferred<any>(); calls.push({ action, args, result }); return result.promise;
  };
  const h = loadHookModule(new URL('./CodexProxySelectionIssues.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../services/codexProxyCatalogService': { ...catalogService,
      setProxyGroupInsecure: invoke('group'), setProxyNodeInsecure: invoke('node'), refreshProxyCatalog: invoke('refresh'),
      getProxyCatalog: invoke('read'),
      cancelProxyCatalog: async (id: string) => { cancelled.push(id); },
    },
    '../ModalErrorMessage': { ModalErrorMessage: () => null },
  });
  const target = { id: 'region', name: 'US', kind: 'url-test', members: ['First', 'Second'], supported: false,
    error: 'PROXY_TLS_INSECURE', insecureNodeIds: ['first', 'second'], issues: [
      { name: 'First', error: 'PROXY_TLS_INSECURE' }, { name: 'Second', error: 'PROXY_TLS_INSECURE' },
    ] };
  const source = { id: 'source', name: 'Subscription', revision: 'revision-1', needsRefresh: true, groups: [target],
    nodes: ['First', 'Second'].map((name) => ({ id: name.toLowerCase(), name, protocol: 'hysteria2', supported: false, insecure: true, error: 'PROXY_TLS_INSECURE' })) };
  const props: any = { source, target, busy: false, onCatalogChange: (value: any) => applied.push(value), onPendingChange: (value: boolean) => pending.push(value) };
  const render = () => h.render(() => h.exports.CodexProxySelectionIssues(props));
  const button = (label: string) => elements(render()).find((element) => element.type === 'button' &&
    (Array.isArray(element.props.children) ? element.props.children : [element.props.children]).includes(label));
  const error = () => elements(render()).find((element) => element.type?.name === 'ModalErrorMessage').props.message;
  return { ...h, props, calls, applied, cancelled, pending, render, button, error };
}

test('viewing certificate and old-record issues does not grant permission or refresh the subscription', async () => {
  const h = harness(); h.render(); await settlePromises();
  assert.equal(h.calls.length, 0);
  assert.equal(h.button('codex.proxy.catalog.allowSelectedOptions').props.disabled, false);
  h.button('codex.proxy.catalog.allowSelectedOptions').props.onClick();
  assert.deepEqual(h.calls[0].args, ['source', 'region', 'revision-1', true]);
  assert.equal(h.calls[0].action, 'group');
  assert.equal(h.button('codex.proxy.catalog.allowSelectedOptions').props.disabled, true);
  h.button('codex.proxy.catalog.allowSelectedOptions').props.onClick();
  assert.equal(h.calls.length, 1, 'a double click must not create two permission transactions');
  const next = { sources: [{ ...h.props.source, revision: 'revision-2' }] };
  h.calls[0].result.resolve(next); await settlePromises();
  assert.deepEqual(h.applied, [next]);
  assert.equal(h.pending[h.pending.length - 1], false);
  assert.equal(h.error(), ''); h.unmount();
});

test('a permission failure stays inline and retry uses the latest source revision', async () => {
  const h = harness(); h.button('codex.proxy.catalog.allowSelectedOptions').props.onClick();
  h.calls[0].result.reject(new Error('CATALOG_CHANGED')); await settlePromises();
  assert.equal(h.error(), catalogService.catalogErrorKey('CATALOG_CHANGED'));
  assert.equal(h.applied.length, 0);
  assert.equal(h.calls[1].action, 'read', 'a stale revision is recovered with a local metadata read, never a subscription download');
  assert.equal(h.button('codex.proxy.catalog.allowSelectedOptions').props.disabled, true);
  const source = { ...h.props.source, revision: 'revision-2' };
  h.calls[1].result.resolve({ sources: [source] }); await settlePromises();
  assert.deepEqual(h.applied, [{ sources: [source] }]);
  h.props.source = source; h.render();
  assert.equal(h.error(), '');
  h.button('codex.proxy.catalog.allowSelectedOptions').props.onClick();
  assert.equal(h.calls[2].args[2], 'revision-2');
  h.calls[2].result.reject(new Error('https://private.invalid/?password=secret')); await settlePromises();
  assert.equal(h.error(), 'codex.proxy.catalog.failed'); h.unmount();
});

test('a source switch ignores the previous permission result and node approval remains node-scoped', async () => {
  const h = harness(); h.button('codex.proxy.catalog.allowSelectedOptions').props.onClick();
  h.props.source = { ...h.props.source, id: 'next-source' }; h.props.target = h.props.source.nodes[0]; h.render();
  h.calls[0].result.resolve({ sources: [] }); await settlePromises();
  assert.deepEqual(h.applied, []);
  h.button('codex.proxy.catalog.allowSelectedOptions').props.onClick();
  assert.equal(h.calls[1].action, 'node');
  assert.deepEqual(h.calls[1].args, ['next-source', 'first', 'revision-1', true]);
  h.unmount(); h.calls[1].result.resolve({ sources: [] }); await settlePromises();
  assert.deepEqual(h.applied, []);
});

test('refresh is explicit, cancellable and cannot overwrite a closed editor', async () => {
  const h = harness(); h.render();
  assert.equal(h.calls.length, 0);
  h.button('common.refresh').props.onClick();
  assert.equal(h.calls[0].action, 'refresh'); assert.equal(h.calls[0].args[1], 'source');
  h.button('common.cancel').props.onClick();
  assert.deepEqual(h.cancelled, [h.calls[0].args[0]]);
  h.unmount(); h.calls[0].result.resolve({ sources: [] }); await settlePromises();
  assert.equal(h.applied.length, 0);
});
