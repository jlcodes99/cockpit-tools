import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { CodexProxyPicker } from '../components/codex/CodexProxyPicker';
import { CodexProxySelect } from '../components/codex/CodexProxySelect';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { defaultProxySelections, proxySelectionGroup, restoreProxySelection, savedRootProxySelections, sourceDefaultDraft } from './codexProxySelection';
import type { ProxyCatalogSource, ProxyCatalogGroup } from '../services/codexProxyCatalogService';
import type { CodexAccount } from '../types/codex';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
import * as selectionUtils from './codexProxySelection';
import * as catalogService from '../services/codexProxyCatalogService';
import * as pickerModel from './codexProxyPickerModel';
import * as previewUtils from './codexProxyPreview';

function elements(tree: any): any[] {
  if (!tree || typeof tree !== 'object') return [];
  return Array.isArray(tree) ? tree.flatMap(elements) : [tree, ...elements(tree.props?.children)];
}

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
const saved = (itemId: string): CodexAccount['egress_proxy'] => ({ protocol: 'RESOURCE', sourceId: source.id, itemId });

test('saved source and node restore even when the bound source is not first', () => {
  const catalog = { sources: [{ ...source, id: 'other' }, source] };
  assert.deepEqual(restoreProxySelection(catalog, saved('node-b')), { sourceId: source.id, itemId: 'node-b', groupId: '' });
  assert.equal(proxySelectionGroup(source, 'node-b'), '');
  assert.deepEqual(restoreProxySelection(catalog, saved('auto')), { sourceId: source.id, itemId: 'auto', groupId: 'auto' });
  assert.equal(proxySelectionGroup(source, 'auto'), 'auto');
});

test('async catalog load restores saved identity without using another account or draft', () => {
  assert.deepEqual(restoreProxySelection({ sources: [] }, saved('node-b')), { sourceId: '', itemId: '', groupId: '' });
  assert.equal(restoreProxySelection({ sources: [source] }, saved('node-b')).itemId, 'node-b');
  assert.equal(restoreProxySelection({ sources: [source] }, saved('node-a')).itemId, 'node-a');
  assert.deepEqual(restoreProxySelection({ sources: [source] }, null), { sourceId: source.id, itemId: '', groupId: '' });
});

test('missing resources never silently select a substitute; unsupported saved nodes still display', () => {
  const catalog = { sources: [source] };
  assert.deepEqual(restoreProxySelection(catalog, { ...saved('node-a')!, sourceId: 'deleted' }), { sourceId: '', itemId: '', groupId: '' });
  assert.deepEqual(restoreProxySelection(catalog, saved('deleted')), { sourceId: source.id, itemId: '', groupId: '' });
  assert.equal(restoreProxySelection(catalog, saved('bad')).itemId, 'bad');
  assert.equal(defaultProxySelections(source, 'bad'), null);
});

test('manual group binding requires an explicit supported member without changing the source', () => {
  const original = JSON.stringify(source);
  assert.equal(defaultProxySelections(source, 'manual'), null);
  assert.deepEqual(defaultProxySelections(source, 'manual', { manual: 'Beta' }), { manual: 'Beta' });
  assert.equal(defaultProxySelections(source, 'manual', { manual: 'Unavailable' }), null);
  assert.equal(defaultProxySelections(source, 'manual', { manual: 'missing' }), null);
  assert.deepEqual(defaultProxySelections(source, 'node-a'), {});
  assert.equal(JSON.stringify(source), original);
});

test('automatic, fallback and balancing groups require choices for nested manual groups', () => {
  for (const kind of ['url-test', 'fallback', 'load-balance']) {
    const nested = { ...source, groups: [group('root', kind, ['manual', 'Alpha']), ...source.groups] };
    assert.equal(defaultProxySelections(nested, 'root'), null);
    assert.deepEqual(defaultProxySelections(nested, 'root', { manual: 'Beta' }), { manual: 'Beta' });
  }
  assert.deepEqual(defaultProxySelections(source, 'auto'), {});
});

test('automatic groups preserve blocking members, without allowing direct routing or standalone built-ins', () => {
  for (const kind of ['url-test', 'fallback', 'load-balance']) {
    const supported = { ...source, groups: [group('root', kind, ['REJECT', 'Alpha', 'REJECT-DROP'])] };
    assert.deepEqual(defaultProxySelections(supported, 'root'), {});
    for (const name of ['DIRECT', 'PASS', 'PASS-RULE', 'COMPATIBLE', 'missing']) {
      const unavailable = { ...source, groups: [group('root', kind, ['Alpha', name])] };
      assert.equal(defaultProxySelections(unavailable, 'root'), null, `${kind} must not accept ${name}`);
    }
  }
  for (const name of ['REJECT', 'REJECT-DROP', 'DIRECT', 'PASS', 'PASS-RULE', 'COMPATIBLE']) {
    assert.equal(defaultProxySelections(source, name), null);
    const builtInNode = { ...source, nodes: [{ id: 'fake-node', name, protocol: 'http', supported: true, error: null }] };
    assert.equal(defaultProxySelections(builtInNode, 'fake-node'), null, 'reserved rules never become standalone nodes');
  }
});

test('manual groups keep explicit blocking choices but reject bypass rules and unsupported members', () => {
  const manual = { ...source, groups: [group('root', 'select', ['REJECT', 'REJECT-DROP', 'DIRECT', 'PASS', 'PASS-RULE', 'COMPATIBLE', 'Unavailable', 'Alpha'])] };
  for (const name of ['REJECT', 'REJECT-DROP']) {
    assert.deepEqual(defaultProxySelections(manual, 'root', { root: name }), { root: name });
  }
  for (const name of ['DIRECT', 'PASS', 'PASS-RULE', 'COMPATIBLE', 'Unavailable']) {
    assert.equal(defaultProxySelections(manual, 'root', { root: name }), null);
  }
  assert.equal(defaultProxySelections({ ...manual, groups: [{ ...manual.groups[0], supported: false, error: 'SUBSCRIPTION_GROUP_OPTIONS' }] }, 'root', { root: 'REJECT' }), null);
});

test('nested manual groups resolve reachable choices, excluding unrelated selectors', () => {
  const nested = { ...source, groups: [group('root', 'select', ['auto']), group('unrelated', 'select', []), ...source.groups] };
  assert.equal(defaultProxySelections(nested, 'root'), null);
  assert.deepEqual(defaultProxySelections(nested, 'root', { root: 'auto' }), { root: 'auto' });
});

test('cycles and unavailable groups fail closed without direct fallback', () => {
  assert.equal(defaultProxySelections({ ...source, groups: [group('cycle', 'select', ['cycle'])] }, 'cycle'), null);
  assert.equal(defaultProxySelections({ ...source, groups: [group('empty', 'select', ['DIRECT', 'Unavailable'])] }, 'empty'), null);
  assert.equal(defaultProxySelections({ ...source, groups: [group('bad-auto', 'url-test', ['Unavailable', 'Alpha'])] }, 'bad-auto'), null);
  assert.equal(defaultProxySelections(source, 'missing'), null);
  assert.deepEqual(defaultProxySelections({ ...source, groups: [group('cycle', 'select', ['cycle', 'Alpha'])] }, 'cycle', { cycle: 'Alpha' }), { cycle: 'Alpha' });
  assert.equal(defaultProxySelections({ ...source, groups: [group('cycle', 'select', ['cycle', 'Alpha'])] }, 'cycle', { cycle: 'cycle' }), null);
});

test('saved manual member can be restored for display without choosing another member', () => {
  assert.deepEqual(savedRootProxySelections(source, { ...saved('manual')!, selectedName: 'Beta' }), { manual: 'Beta' });
  assert.deepEqual(savedRootProxySelections(source, { ...saved('manual')!, selectedName: 'missing' }), {});
});

test('a source default only prefills a resolvable draft and never invents a choice', () => {
  const withDefault = (itemId: string, selections: Record<string, string> = {}, groupId: string | null = null) =>
    ({ ...source, default: { itemId, groupId, selections }, defaultInvalidated: false });
  assert.equal(sourceDefaultDraft(undefined), null);
  assert.equal(sourceDefaultDraft(source), null);
  assert.deepEqual(sourceDefaultDraft(withDefault('node-b', {}, 'auto')), { itemId: 'node-b', groupId: 'auto', selections: {} });
  assert.deepEqual(sourceDefaultDraft(withDefault('manual', { manual: 'Beta' })), { itemId: 'manual', groupId: 'manual', selections: { manual: 'Beta' } });
  assert.equal(sourceDefaultDraft(withDefault('manual')), null, '手动组没有成员时不得自动挑选');
  assert.equal(sourceDefaultDraft(withDefault('manual', { manual: 'Unavailable' })), null);
  assert.equal(sourceDefaultDraft(withDefault('bad')), null, '已失效节点不得带入草稿');
  assert.equal(sourceDefaultDraft(withDefault('deleted')), null, '已删除节点不得静默替换');
  assert.equal(sourceDefaultDraft(withDefault('node-b', {}, 'removed'))?.groupId, '', '过期的分组上下文不得继续使用');
  assert.equal(sourceDefaultDraft(withDefault('auto'))?.groupId, 'auto');
});

test('default commands keep the IPC contract for source ids, item ids and member choices', async () => {
  const calls: unknown[] = [];
  const compiledService = ts.transpileModule(readFileSync(new URL('../services/codexProxyCatalogService.ts', import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports: Record<string, any> = {};
  vm.runInNewContext(compiledService, {
    exports, setTimeout, clearTimeout,
    require: () => ({ invoke: async (command: string, args?: unknown) => { calls.push({ command, args }); return { sources: [] }; } }),
  });
  await exports.setProxyCatalogDefault('subscription', 'manual', { manual: 'Beta' });
  await exports.setProxyCatalogDefault('subscription', 'node-b', {}, 'auto');
  await exports.clearProxyCatalogDefault('subscription');
  // Cross-realm objects keep sandbox prototypes, so compare the serialized contract.
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'codex_proxy_catalog_set_default', args: { sourceId: 'subscription', itemId: 'manual', selections: { manual: 'Beta' } } },
    { command: 'codex_proxy_catalog_set_default', args: { sourceId: 'subscription', itemId: 'node-b', selections: {}, groupId: 'auto' } },
    { command: 'codex_proxy_catalog_clear_default', args: { sourceId: 'subscription' } },
  ]));
});

test('picker renders restored node and group policy alongside its current selection card', async () => {
  const i18n = createInstance();
  await i18n.init({ lng: 'en', resources: { en: { translation: { codex: { proxy: { catalog: {
    groups: 'Groups', nodes: 'Nodes', groupPolicyChoice: 'Group policy', measureGroup: 'Test group',
  } } } } } } });
  const latency = { results: {}, running: false, total: 0, completed: 0, errorKey: '', measure: () => {}, cancel: () => {} };
  const render = (itemId: string) => renderToStaticMarkup(createElement(I18nextProvider, { i18n },
    createElement(CodexProxyPicker, { source, itemId, selectedGroupId: 'manual', selections: {}, busy: false, latency, choose: () => {}, chooseMember: () => {} })));
  const nodeHtml = render('node-b');
  assert.match(nodeHtml, /<span>manual<\/span>/);
  assert.match(nodeHtml, /<span>Beta<\/span>/);
  assert.match(nodeHtml, /class="codex-picker-current"/);
  assert.match(nodeHtml, /aria-label="Test group"/);
  assert.doesNotMatch(nodeHtml, /codex-picker-summary|codex-picker-path|useGroupPolicy|chooseMember/);
  const groupHtml = render('auto');
  assert.match(groupHtml, /<span>auto<\/span>/);
  assert.match(groupHtml, /<span>Group policy<\/span>/);
});

test('picker keeps manual group identity while choosing its member; standalone nodes remain separate', () => {
  // Exercise component event handlers without a browser or GUI automation.
  const h = loadHookModule(new URL('../components/codex/CodexProxyPicker.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../services/codexProxyCatalogService': catalogService,
    '../../utils/codexProxySelection': selectionUtils,
    '../../utils/codexProxyPickerModel': pickerModel,
    '../../utils/codexProxyPreview': previewUtils,
    './CodexProxySelect': { CodexProxySelect },
  });
  let itemId = ''; let selectedGroupId = ''; let selections: Record<string, string> = {}; const chosen: string[] = [];
  const measurements: { ids: string[]; onlyStale?: boolean; groupId?: string }[] = [];
  const latency = { results: {}, running: false, total: 0, completed: 0, errorKey: '',
    measure: (ids: string[], onlyStale?: boolean, groupId?: string) => measurements.push({ ids: [...ids], onlyStale, groupId }), cancel: () => {} };
  const render = () => h.render(() => h.exports.CodexProxyPicker({ source, itemId, selectedGroupId, selections, busy: false, latency,
      choose: (id: string, nextGroup: string) => { itemId = id; selectedGroupId = nextGroup; selections = {}; chosen.push(id); },
      chooseMember: (groupId: string, member: string) => { selections = { ...selections, [groupId]: member }; } }));
  const select = (placeholder: string) => {
    const entry = elements(render()).find((element) => element.type === CodexProxySelect && element.props.placeholder === `codex.proxy.catalog.${placeholder}`);
    assert.ok(entry, `missing ${placeholder} selector`);
    return entry;
  };
  select('groups').props.onChange('manual');
  assert.equal(itemId, 'manual');
  assert.equal(defaultProxySelections(source, itemId, selections), null);
  assert.deepEqual(measurements, [{ ids: ['node-b', 'node-a'], onlyStale: true, groupId: 'manual' }], 'only the explicitly chosen group is checked');
  assert.equal(select('groups').props.value, 'manual');
  elements(render()).find((element) => element.props?.className?.includes('codex-picker-icon-button')).props.onClick();
  assert.deepEqual(measurements[1], { ids: ['node-b', 'node-a'], onlyStale: false, groupId: 'manual' });
  assert.deepEqual(chosen, ['manual']);
  select('chooseMember').props.onChange('Beta');
  assert.equal(itemId, 'manual');
  assert.deepEqual(defaultProxySelections(source, itemId, selections), { manual: 'Beta' });
  assert.deepEqual(measurements[2], { ids: ['node-b'], onlyStale: true, groupId: 'manual' });
  select('groups').props.onChange('');
  assert.deepEqual(measurements[3], { ids: [], onlyStale: true, groupId: undefined }, 'clearing the group restores standalone cache without testing nodes');
  select('nodes').props.onChange('node-a');
  assert.equal(itemId, 'node-a');
  assert.equal(select('nodes').props.value, 'node-a');
  assert.deepEqual(defaultProxySelections(source, itemId, selections), {});
  assert.deepEqual(measurements[4], { ids: ['node-a'], onlyStale: true, groupId: undefined });
  h.unmount();
});

test('shared nodes restore the recorded group, independent of subscription order', () => {
  const binding = { ...saved('node-b')!, groupId: 'auto' };
  assert.equal(restoreProxySelection({ sources: [source] }, binding).groupId, 'auto');
  assert.equal(proxySelectionGroup({ ...source, groups: [...source.groups].reverse() }, 'node-b', 'manual'), 'manual');
  assert.equal(proxySelectionGroup(source, 'node-b'), '');
  assert.equal(proxySelectionGroup(source, 'node-b', 'removed'), '');
  assert.equal(proxySelectionGroup({ ...source, groups: [group('auto', 'url-test', ['Alpha'])] }, 'node-b', 'auto'), '');
  assert.equal(proxySelectionGroup(source, 'auto'), 'auto');
});

test('a chosen subgroup restores its explicit parent for editing and source defaults', () => {
  const nested = { ...source, groups: [group('region', 'url-test', ['auto', 'Alpha']), group('other-region', 'select', ['auto']), ...source.groups] };
  const binding = { ...saved('auto')!, groupId: 'region' };
  assert.deepEqual(restoreProxySelection({ sources: [nested] }, binding), { sourceId: source.id, itemId: 'auto', groupId: 'region' });
  assert.equal(proxySelectionGroup(nested, 'auto', 'other-region'), 'other-region');
  assert.equal(proxySelectionGroup(nested, 'auto', 'missing'), 'auto');
  assert.equal(proxySelectionGroup(nested, 'auto', 'manual'), 'auto', 'unrelated groups cannot replace the parent');
  assert.deepEqual(sourceDefaultDraft({ ...nested, default: { itemId: 'auto', groupId: 'region', selections: {} } }), {
    itemId: 'auto', groupId: 'region', selections: {},
  });
});
