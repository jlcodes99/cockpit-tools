import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import * as strategyService from '../../services/codexProxyStrategyService';
import * as catalogService from '../../services/codexProxyCatalogService';
import type { ProxyCatalog, ProxyCatalogSource } from '../../services/codexProxyCatalogService';

function elements(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  return value && typeof value === 'object' && value.props ? [value, ...elements(value.props.children)] : [];
}

function harness(onlyBlocked = false) {
  const nodes = Array.from({ length: 50 }, (_, index) => ({
    id: `blocked-${index}`, name: `Region ${index}`, protocol: index < 27 ? 'anytls' : 'hysteria2',
    supported: false, insecure: true, error: 'PROXY_TLS_INSECURE',
  }));
  const subscription: ProxyCatalogSource = {
    id: 'subscription', name: 'Subscription', kind: 'subscription', revision: 'revision-1',
    nodes: onlyBlocked ? nodes : [...nodes, { id: 'website', name: 'Website', protocol: 'vmess', supported: true, insecure: false, error: null }],
    groups: [], updatedAt: 1, lastAttemptAt: null, autoUpdate: false, error: null, default: null, defaultInvalidated: false,
  };
  const permissionCalls: { args: any[]; result: ReturnType<typeof deferred<ProxyCatalog>> }[] = [];
  const saves: any[] = [];
  const applied: ProxyCatalog[] = [];
  let closed = 0;
  const imports = {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../ModalErrorMessage': { ModalErrorMessage: 'error' },
    '../../utils/codexProxyRemoval': {},
    '../../services/codexProxyCatalogService': {
      ...catalogService,
      setProxyNodeInsecure(...args: any[]) { const result = deferred<ProxyCatalog>(); permissionCalls.push({ args, result }); return result.promise; },
    },
  };
  const parent = loadHookModule(new URL('./CodexProxyStrategyPanel.tsx', import.meta.url), {
    ...imports,
    'react-dom': { createPortal: (value: unknown) => value },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    './useCodexProxyExitEditor': { useCodexProxyAccountName: () => (value: string) => value },
    './CodexProxySelectionIssues': { CodexProxySelectionIssues: 'issues' },
    '../SingleSelectFilterDropdown': { SingleSelectFilterDropdown: 'filter' },
    '../../services/codexProxyStrategyService': {
      ...strategyService,
      async saveProxyStrategy(value: any) { saves.push(value); return { sources: [] }; },
    },
  }, { document: { body: {} }, requestAnimationFrame: () => 1, cancelAnimationFrame() {} });
  const child = loadHookModule(new URL('./CodexProxySelectionIssues.tsx', import.meta.url), imports);
  const props = {
    source: null, sources: [subscription], busy: false,
    onClose() { closed++; }, onSaved() { closed++; },
    onCatalogChange(next: ProxyCatalog) { applied.push(next); props.sources = next.sources; },
  };
  const render = () => parent.render(() => parent.exports.StrategyEditorDialog(props));
  const tree = () => elements(render());
  const candidates = () => tree().filter((entry) => entry.props.className === 'codex-strategy-candidate');
  const candidate = (name: string) => candidates().find((entry) => elements(entry).some((child) => child.type === 'strong' && child.props.children === name))!;
  const selected = () => tree().find((entry) => entry.type === 'ol').props.children.length;
  const save = () => tree().find((entry) => entry.type === 'button' && entry.props.className === 'btn btn-primary')!.props;
  const issueProps = () => tree().find((entry) => entry.type === 'issues')?.props;
  const issueTree = () => elements(child.render(() => child.exports.CodexProxySelectionIssues(issueProps())));
  const allow = () => issueTree().find((entry) => entry.type === 'button' && elements(entry).length && entry.props.children.includes('codex.proxy.catalog.allowSelectedOptions'))!.props;
  const approved = (id: string): ProxyCatalog => ({ sources: [{ ...subscription, revision: 'revision-2',
    nodes: subscription.nodes.map((entry) => entry.id === id ? { ...entry, supported: true, error: null } : entry),
  }] });
  return { parent, child, props, tree, candidates, candidate, selected, save, issueProps, issueTree, allow, approved,
    permissionCalls, saves, applied, closed: () => closed, unmount() { child.unmount(); parent.unmount(); } };
}

test('strategy members show all 51 nodes and require explicit node permission before adding', async () => {
  const h = harness();
  assert.equal(h.candidates().length, 51);
  assert.equal(h.permissionCalls.length, 0);
  h.candidate('Website').props.onClick();
  h.candidate('Region 49').props.onClick();
  assert.equal(h.selected(), 1, 'inspection must not add an unauthorized node');
  assert.equal(h.issueProps().target.id, 'blocked-49');
  assert.equal(h.issueProps().source.revision, 'revision-1');
  h.issueTree();
  assert.equal(h.permissionCalls.length, 0, 'opening permission details must not authorize');
  h.allow().onClick();
  assert.deepEqual(h.permissionCalls[0].args, ['subscription', 'blocked-49', 'revision-1', true]);
  assert.equal(h.save().disabled, true);
  h.save().onClick();
  await settlePromises();
  assert.equal(h.saves.length, 0, 'strategy saves wait for a permission transaction');
  h.permissionCalls[0].result.resolve(h.approved('blocked-49'));
  await settlePromises();
  assert.equal(h.closed(), 0, 'granting permission keeps the strategy editor open');
  assert.equal(h.selected(), 1, 'granting permission preserves the draft and does not add automatically');
  h.candidate('Region 49').props.onClick();
  assert.equal(h.selected(), 2);
  h.tree().find((entry) => entry.type === 'input' && entry.props.className === 'codex-strategy-input').props.onChange({ target: { value: 'My group' } });
  h.save().onClick(); await settlePromises();
  assert.deepEqual(JSON.parse(JSON.stringify(h.saves[0].members)), [
    { sourceId: 'subscription', itemId: 'website', name: 'Website', sourceName: 'Subscription' },
    { sourceId: 'subscription', itemId: 'blocked-49', name: 'Region 49', sourceName: 'Subscription' },
  ]);
  h.unmount();
});

test('failed permission stays inside the editor and leaves the node unavailable for retry', async () => {
  const h = harness(); h.candidate('Region 0').props.onClick(); h.allow().onClick();
  h.permissionCalls[0].result.reject(new Error('CATALOG_DOWNLOAD'));
  await settlePromises();
  assert.equal(h.issueTree().find((entry) => entry.type === 'error')!.props.message, 'codex.proxy.catalog.downloadFailed');
  assert.equal(h.selected(), 0);
  assert.equal(h.applied.length, 0);
  assert.equal(h.closed(), 0);
  assert.equal(h.save().disabled, false);
  h.allow().onClick(); assert.equal(h.permissionCalls.length, 2);
  assert.equal(h.issueTree().find((entry) => entry.type === 'error')!.props.message, '');
  h.unmount();
  h.permissionCalls[1].result.resolve(h.approved('blocked-0')); await settlePromises();
  assert.equal(h.applied.length, 0, 'late permission results must not update a closed editor');
});

test('protocol failures stay visible for inspection but cannot become strategy members', () => {
  const h = harness();
  h.props.sources = [{ ...h.props.sources[0], nodes: [{ id: 'bad', name: 'Bad node', protocol: 'unsupported', supported: false, error: 'PROXY_UNSUPPORTED_OPTION' }] }];
  h.candidate('Bad node').props.onClick();
  assert.equal(h.selected(), 0);
  assert.equal(h.issueTree().some((entry) => entry.type === 'button'), false, 'protocol errors have no permission override');
  h.unmount();
});

test('a subscription containing only permission-blocked nodes can still open the strategy editor', () => {
  const h = harness(true);
  const panel = loadHookModule(new URL('./CodexProxyStrategyPanel.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexProxyRemoval': {},
    './useCodexProxyExitEditor': {},
    '../SingleSelectFilterDropdown': {},
    './CodexProxySelectionIssues': {},
  });
  const output = panel.render(() => panel.exports.CodexProxyStrategyPanel({ sources: h.props.sources, busy: false, onCatalog() {}, onRemoved() {} }));
  const create = elements(output).find((entry) => entry.type === 'button' && entry.props.children.includes('codex.proxy.catalog.strategyCreate'))!;
  assert.equal(create.props.disabled, false);
  assert.equal(h.candidates().length, 50);
  panel.unmount(); h.unmount();
});
