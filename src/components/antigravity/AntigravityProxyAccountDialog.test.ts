import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { restoreExitChoice, sameExitChoice } from '../../utils/codexProxyDraft';
import { defaultProxySelections, sourceDefaultDraft } from '../../utils/codexProxySelection';
import type { AntigravityProxyView } from '../../services/antigravityProxyService';
import type { ProxyCatalog } from '../../services/codexProxyCatalogService';

function elements(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  return value?.props ? [value, ...elements(value.props.children)] : [];
}
const catalog: ProxyCatalog = { sources: [{ id: 'source', name: 'Shared', kind: 'manual', revision: '1', updatedAt: 0,
  lastAttemptAt: null, autoUpdate: false, error: null, default: null, defaultInvalidated: false,
  groups: [], nodes: [{ id: 'node', name: 'Hong Kong', protocol: 'http', supported: true, error: null }] }] };
const saved = (accountId = 'a'): AntigravityProxyView => ({ accountId, binding: null, error: null, legacyBound: false });

function setup(options: { read?: Promise<AntigravityProxyView>; catalog?: ProxyCatalog; bind?: Promise<AntigravityProxyView> } = {}) {
  const noop = () => {};
  const accepted: AntigravityProxyView[] = [];
  const accept = (view: AntigravityProxyView) => accepted.push(view);
  const writes: unknown[][] = [];
  let closed = 0;
  const t = (key: string) => key;
  const imports: Record<string, unknown> = {
    'react-dom': { createPortal: (value: unknown) => value },
    'react-i18next': { useTranslation: () => ({ t }) },
    '../../services/antigravityProxyService': { antigravityProxyApi: {
      get: () => options.read ?? Promise.resolve(saved()),
      bind: (...args: unknown[]) => { writes.push(args); return options.bind ?? Promise.resolve(saved()); },
      clear: (...args: unknown[]) => { writes.push(['clear', ...args]); return Promise.resolve(saved()); },
    } },
    '../../stores/useAntigravityProxyStore': { useAntigravityProxyStore: Object.assign((selector: any) => selector({ accept }), { getState: () => ({ reload: async () => {} }) }) },
    '../../services/codexProxyCatalogService': { getProxyCatalog: async () => options.catalog ?? catalog, catalogErrorKey: () => 'safe-error', cancelProxyCatalog: async () => {} },
    '../../utils/codexProxyDraft': { restoreExitChoice, sameExitChoice },
    '../../utils/codexProxySelection': { defaultProxySelections, sourceDefaultDraft },
    '../../utils/codexProxyPickerModel': { proxySourceInspectable: () => true },
    '../../utils/codexProxyPresentation': { proxySummary: () => 'Saved node' },
    '../../hooks/useEscClose': { useEscCloseTopmost: noop },
    '../../hooks/useModalScrollLock': { useModalScrollLock: noop },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: noop },
    '../codex/useProxyLatency': { useProxyLatency: () => ({ running: false, cancel: noop }) },
    '../codex/useCodexProxyExitEditor': { useCodexProxyAccountName: () => () => 'Account' },
  };
  for (const [path, name] of [['../SingleSelectDropdown', 'SingleSelectDropdown'], ['../ModalErrorMessage', 'ModalErrorMessage'],
    ['../codex/CodexProxyPicker', 'CodexProxyPicker'], ['../codex/CodexProxyResources', 'CodexProxyResources']]) imports[path] = { [name]: name };
  const h = loadHookModule(new URL('./AntigravityProxyAccountDialog.tsx', import.meta.url), imports, { document: { body: {} } });
  h.render(() => h.exports.AntigravityProxyAccountDialog({ account: { id: 'a', email: 'a@example.invalid' }, onClose: () => closed++ }));
  const tree = () => elements(h.flush());
  return { h, tree, writes, accepted, closed: () => closed, modes: () => tree().filter((node) => node.props.role === 'radio'),
    picker: () => tree().find((node) => node.type === 'CodexProxyPicker'),
    save: () => tree().find((node) => node.type === 'button' && node.props.className === 'btn btn-primary' && Array.isArray(node.props.children)) };
}

test('independent mode renders the shared picker; choosing is only a draft until Save', async () => {
  const d = setup(); await settlePromises(); d.h.flush();
  d.modes()[1].props.onClick(); d.h.flush();
  assert.ok(d.picker());
  d.picker().props.choose('node', ''); d.h.flush();
  assert.equal(d.writes.length, 0);
  assert.equal(d.save().props.disabled, false);
  await d.save().props.onClick(); await settlePromises();
  assert.deepEqual(d.writes[0], ['a', 'source', 'node', {}, '']);
  assert.equal(d.closed(), 1); d.h.unmount();
});

test('empty shared catalog leads to the existing resource manager, not a URL input', async () => {
  const d = setup({ catalog: { sources: [] } }); await settlePromises(); d.h.flush(); d.modes()[1].props.onClick();
  const go = d.tree().find((node) => node.type === 'button' && node.props.children === 'codex.proxy.manager.proxies');
  assert.ok(go); go.props.onClick();
  assert.ok(d.tree().some((node) => node.type === 'CodexProxyResources'));
  assert.equal(d.tree().some((node) => node.type === 'input'), false); d.h.unmount();
});

test('a read completing after unmount cannot publish to the account cache', async () => {
  const read = deferred<AntigravityProxyView>(); const d = setup({ read: read.promise }); d.h.unmount();
  read.resolve(saved()); await settlePromises(); assert.equal(d.accepted.length, 0);
});

test('explicitly reselecting the saved node can refresh its frozen snapshot', async () => {
  const existing = { ...saved(), binding: { protocol: 'http', sourceId: 'source', itemId: 'node', name: 'Hong Kong' } };
  const d = setup({ read: Promise.resolve(existing) }); await settlePromises(); d.h.flush();
  assert.equal(d.save().props.disabled, true);
  d.picker().props.choose('node', ''); d.h.flush();
  assert.equal(d.save().props.disabled, false);
  d.save().props.onClick(); await settlePromises();
  assert.equal(d.writes.length, 1); d.h.unmount();
});

test('following Settings clears only the Antigravity account binding', async () => {
  const d = setup({ read: Promise.resolve({ ...saved(), legacyBound: true }) }); await settlePromises(); d.h.flush();
  d.modes()[0].props.onClick(); d.save().props.onClick(); await settlePromises();
  assert.deepEqual(d.writes, [['clear', 'a']]); d.h.unmount();
});

test('double Save submits once; failure preserves the selected draft and sanitizes errors', async () => {
  const result = deferred<AntigravityProxyView>(); const d = setup({ bind: result.promise });
  await settlePromises(); d.h.flush(); d.modes()[1].props.onClick(); d.picker().props.choose('node', '');
  const save = d.save().props.onClick; save(); save();
  assert.equal(d.writes.length, 1);
  result.reject(new Error('http://user:password@secret.invalid')); await settlePromises();
  assert.equal(d.picker().props.itemId, 'node');
  assert.equal(d.closed(), 0);
  assert.equal(d.tree().find((node) => node.type === 'ModalErrorMessage').props.message, 'safe-error');
  d.h.unmount();
});
