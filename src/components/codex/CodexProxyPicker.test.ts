import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';
import { CodexProxyPicker } from './CodexProxyPicker';
import { CodexProxySelect } from './CodexProxySelect';
import { CodexProxySelectionIssues } from './CodexProxySelectionIssues';
import { CodexProxyResourceNodes } from './CodexProxyResourceNodes';
import * as catalogService from '../../services/codexProxyCatalogService';
import * as selectionUtils from '../../utils/codexProxySelection';
import * as pickerModel from '../../utils/codexProxyPickerModel';
import * as previewUtils from '../../utils/codexProxyPreview';
import type { ProxyCatalogGroup, ProxyCatalogSource } from '../../services/codexProxyCatalogService';
import type { CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';
import en from '../../locales/en-US.json';

const group = (id: string, name: string, kind: string, members: string[]): ProxyCatalogGroup => ({ id, name, kind, members, supported: true, error: null });
const source: ProxyCatalogSource = {
  id: 'source', name: 'Example', kind: 'subscription', revision: '1', updatedAt: 0, lastAttemptAt: null,
  autoUpdate: false, error: null, default: null, defaultInvalidated: false,
  nodes: [{ id: 'us', name: 'US node', protocol: 'http', supported: true, error: null }],
  groups: [group('region', 'US group', 'select', ['Automatic', 'US node', 'REJECT', 'REJECT-DROP', 'DIRECT', 'PASS', 'PASS-RULE', 'COMPATIBLE']),
    group('auto', 'Automatic', 'url-test', ['US node', 'REJECT']),
    { ...group('unsupported', 'Unsupported group', 'url-test', ['US node', 'Missing']), supported: false, error: 'SUBSCRIPTION_GROUP_UNAVAILABLE' }],
};
const latency = { results: {}, running: false, total: 0, completed: 0, errorKey: '', measure: () => {}, cancel: () => {} };

async function markup(data: ProxyCatalogSource, itemId: string, selectedGroupId: string, selections: Record<string, string> = {}, extra: Partial<Parameters<typeof CodexProxyPicker>[0]> = {}) {
  const i18n = createInstance();
  await i18n.init({ lng: 'en', resources: { en: { translation: en } } });
  return renderToStaticMarkup(createElement(I18nextProvider, { i18n }, createElement(CodexProxyPicker, {
    source: data, itemId, selectedGroupId, selections, busy: false, latency, choose: () => {}, chooseMember: () => {}, ...extra,
  })));
}

function elements(tree: any): any[] {
  if (!tree || typeof tree !== 'object') return [];
  if (Array.isArray(tree)) return tree.flatMap(elements);
  return [tree, ...elements(tree.props?.children)];
}

function pickerModule() {
  return loadHookModule(new URL('./CodexProxyPicker.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../services/codexProxyCatalogService': catalogService,
    '../../utils/codexProxySelection': selectionUtils,
    '../../utils/codexProxyPickerModel': pickerModel,
    '../../utils/codexProxyPreview': previewUtils,
    './CodexProxySelect': { CodexProxySelect },
    './CodexProxySelectionIssues': { CodexProxySelectionIssues },
    '../ModalErrorMessage': { ModalErrorMessage: () => null },
  });
}

function pickerSelect(tree: any, placeholder: string) {
  const select = elements(tree).find((entry) => entry.type === CodexProxySelect && entry.props.placeholder === `codex.proxy.catalog.${placeholder}`);
  assert.ok(select, `missing ${placeholder} selector`);
  return select;
}

test('selecting an automatic member renders its name, policy and parent without requiring a node', async () => {
  const html = await markup(source, 'region', 'region', { region: 'Automatic' });
  assert.match(html, /<span>US group<\/span>/);
  assert.match(html, /<span>Automatic<\/span>/);
  assert.match(html, /Lowest latency/);
  assert.match(html, /Use the whole group without pinning a node/);
  assert.doesNotMatch(html, /No specific node|role="alert"/);
  assert.deepEqual(selectionUtils.defaultProxySelections(source, 'region', { region: 'Automatic' }), { region: 'Automatic' });
});

test('a directly selected child policy retains the parent group and child name after restoration', async () => {
  const data = { ...source, groups: [{ ...source.groups[0], kind: 'fallback' }, ...source.groups.slice(1)] };
  const html = await markup(data, 'auto', 'region');
  assert.match(html, /<span>US group<\/span>/);
  assert.match(html, /<span>Automatic<\/span>/);
  assert.match(html, /Lowest latency/);
  assert.doesNotMatch(html, /<span>Group policy<\/span>/);
  assert.deepEqual(selectionUtils.defaultProxySelections(data, 'auto'), {});
});

test('an unavailable group displays a sanitized inline reason; choosing a valid node clears it', async () => {
  const invalid = await markup(source, 'unsupported', 'unsupported');
  assert.match(invalid, /role="alert"/);
  assert.match(invalid, /Group members failed validation\. Check the individual reasons/);
  const detailedSource = { ...source, groups: [{ ...source.groups[2], error: 'SUBSCRIPTION_GROUP_MEMBER_MISSING',
    issues: [{ name: 'Missing', error: 'SUBSCRIPTION_GROUP_MEMBER_MISSING' }],
  }] };
  const detailed = await markup(detailedSource, 'unsupported', 'unsupported');
  assert.match(detailed, /Group member issues/);
  assert.match(detailed, /<strong>Missing<\/strong>/);
  assert.match(detailed, /This member was not found in the subscription/);
  assert.doesNotMatch(detailed, /form a cycle|Group members failed validation/);
  const unsafe = { ...source, groups: [{ ...source.groups[2], error: 'https://private.invalid/?token=secret' }] };
  const sanitized = await markup(unsafe, 'unsupported', 'unsupported');
  assert.doesNotMatch(sanitized, /private\.invalid|token=secret/);
  assert.match(sanitized, /This configuration is unavailable/);
  const valid = await markup(source, 'us', 'region');
  assert.doesNotMatch(valid, /role="alert"/);
  assert.doesNotMatch(valid, /Use the whole group without pinning a node/);
});

test('an explicit blocking member keeps its explanation visible after the menu closes and clears on a node choice', async () => {
  for (const member of ['REJECT', 'REJECT-DROP']) {
    const html = await markup(source, 'region', 'region', { region: member });
    assert.match(html, /role="status"/);
    assert.match(html, /This member blocks requests without falling back to a direct connection/);
    assert.doesNotMatch(html, /role="alert"/);
    assert.deepEqual(selectionUtils.defaultProxySelections(source, 'region', { region: member }), { region: member });
  }
  const valid = await markup(source, 'region', 'region', { region: 'US node' });
  assert.doesNotMatch(valid, /This member blocks requests/);
  const automatic = await markup(source, 'region', 'region', { region: 'Automatic' });
  assert.doesNotMatch(automatic, /This member blocks requests/, 'a blocking candidate in an automatic group is not an explicit block');
  const bypass = await markup(source, 'region', 'region', { region: 'PASS-RULE' });
  assert.match(bypass, /role="alert"/);
  assert.equal(selectionUtils.defaultProxySelections(source, 'region', { region: 'PASS-RULE' }), null);
});

test('picker handlers retain a child policy identity and never silently choose a leaf node', () => {
  const data = { ...source, groups: [{ ...source.groups[0], kind: 'fallback' }, ...source.groups.slice(1)] };
  const h = pickerModule();
  let itemId = 'region'; let selectedGroupId = 'region';
  const props = () => ({ source: data, itemId, selectedGroupId, selections: {}, busy: false, latency,
    choose: (id: string, parent: string) => { itemId = id; selectedGroupId = parent; }, chooseMember: () => {},
  });
  const render = () => h.render(() => h.exports.CodexProxyPicker(props()));
  pickerSelect(render(), 'groupPolicyChoice').props.onChange('auto');
  assert.equal(itemId, 'auto');
  assert.equal(selectedGroupId, 'region');
  const selected = render();
  assert.equal(pickerSelect(selected, 'groups').props.value, 'region');
  assert.equal(pickerSelect(selected, 'groupPolicyChoice').props.value, 'auto');
  pickerSelect(selected, 'groupPolicyChoice').props.onChange('');
  assert.equal(itemId, 'region');
  assert.equal(selectedGroupId, 'region');
  h.unmount();
});

test('manual member options allow explicit blocking rules but keep every bypass rule disabled', () => {
  const h = pickerModule();
  const tree = h.render(() => h.exports.CodexProxyPicker({ source, itemId: 'region', selectedGroupId: 'region', selections: {}, busy: false, latency, choose: () => {}, chooseMember: () => {} }));
  const options = pickerSelect(tree, 'chooseMember').props.options;
  for (const name of ['REJECT', 'REJECT-DROP']) {
    const option = options.find((entry: any) => entry.value === name);
    assert.equal(option.disabled, false);
    assert.equal(option.detail, 'codex.proxy.catalog.blockingMemberHint');
  }
  for (const name of ['DIRECT', 'PASS', 'PASS-RULE', 'COMPATIBLE']) assert.equal(options.find((entry: any) => entry.value === name).disabled, true);
  h.unmount();
});

const scopedSource: ProxyCatalogSource = { ...source,
  nodes: [...source.nodes, { ...source.nodes[0], id: 'us2', name: 'US node 2' }, { ...source.nodes[0], id: 'jp', name: 'Japan node' },
    { ...source.nodes[0], id: 'bad', name: 'Bad', supported: false }],
  groups: [group('global', 'Global', 'fallback', ['US group', 'Japan']),
    group('region', 'US group', 'select', ['Automatic', 'US node', 'Bad', 'REJECT']),
    group('auto', 'Automatic', 'url-test', ['Nested', 'US node']),
    group('nested', 'Nested', 'fallback', ['US node 2', 'Bad']),
    group('japan', 'Japan', 'url-test', ['Japan node'])],
};

function latencyPickerHarness() {
  const measured: { ids: string[]; onlyStale?: boolean; groupId?: string }[] = []; const events: string[] = [];
  const h = pickerModule();
  const props: Parameters<typeof CodexProxyPicker>[0] = {
    source: scopedSource, itemId: 'global', selectedGroupId: 'global', selections: {}, busy: false,
    latency: { ...latency, running: true,
      measure: (ids, onlyStale, groupId) => { measured.push({ ids: [...ids], onlyStale, groupId }); events.push('measure'); },
      cancel: () => { events.push('cancel'); },
    },
    choose: (id, parent) => { props.itemId = id; props.selectedGroupId = parent; props.selections = {}; },
    chooseMember: (id, name) => { props.selections = { ...props.selections, [id]: name }; },
  };
  const render = () => h.render(() => h.exports.CodexProxyPicker(props));
  const selects = () => elements(render()).filter((entry) => entry.type === CodexProxySelect);
  const policySelect = () => pickerSelect(render(), 'groupPolicyChoice');
  const groupSelect = () => pickerSelect(render(), 'groups');
  const manualSelect = () => pickerSelect(render(), 'chooseMember');
  const nodeSelect = () => pickerSelect(render(), 'nodes');
  return { ...h, props, measured, events, render, selects, policySelect, groupSelect, manualSelect, nodeSelect };
}

test('opening, restoring, refreshing and re-enabling a picker never starts latency checks', () => {
  const h = latencyPickerHarness();
  h.render(); h.render();
  h.props.itemId = 'region'; h.props.selectedGroupId = 'region'; h.props.selections = { region: 'Automatic' };
  h.render();
  h.props.busy = true; h.render();
  h.props.busy = false; h.render();
  h.props.source = { ...scopedSource, revision: '2' }; h.render();
  h.props.source = { ...scopedSource, id: 'other' }; h.render();
  assert.deepEqual(h.measured, [], 'only explicit user selections may start probes');
  h.unmount();
});

test('clicking a group tests its supported descendants, cancels previous checks and leaves selectors usable', () => {
  const h = latencyPickerHarness();
  const selectors = h.selects();
  assert.ok(selectors.every((entry) => entry.props.disabled === false));
  h.groupSelect().props.onChange('region');
  assert.deepEqual(h.measured, [{ ids: ['us2', 'us'], onlyStale: true, groupId: 'region' }]);
  assert.deepEqual(h.events, ['cancel', 'measure']);
  h.render(); assert.equal(h.measured.length, 1, 'restoring the chosen group must not schedule another batch');
  h.groupSelect().props.onChange('japan');
  assert.deepEqual(h.measured[1], { ids: ['jp'], onlyStale: true, groupId: 'japan' });
  assert.deepEqual(h.events, ['cancel', 'measure', 'cancel', 'measure']);
  h.groupSelect().props.onChange('');
  assert.deepEqual(h.measured[2], { ids: [], onlyStale: true, groupId: undefined }, 'browsing all nodes switches cache without probing the source');
  h.unmount(); assert.equal(h.events[h.events.length - 1], 'cancel');
});

test('choosing child groups and nodes never retests the browsing parent or its siblings', () => {
  const h = latencyPickerHarness();
  h.policySelect().props.onChange('region');
  assert.equal(h.props.selectedGroupId, 'global');
  assert.deepEqual(h.measured[0], { ids: ['us2', 'us'], onlyStale: true, groupId: 'region' });
  h.manualSelect().props.onChange('Automatic');
  assert.deepEqual(h.measured[1], { ids: ['us2', 'us'], onlyStale: true, groupId: 'auto' });
  h.manualSelect().props.onChange('US node');
  assert.deepEqual(h.measured[2], { ids: ['us'], onlyStale: true, groupId: 'region' });
  h.manualSelect().props.onChange('REJECT');
  assert.deepEqual(h.measured[3], { ids: [], onlyStale: true, groupId: 'region' }, 'blocking members must not expand to their parent');
  h.policySelect().props.onChange('');
  assert.deepEqual(h.measured[4], { ids: ['us2', 'us', 'jp'], onlyStale: true, groupId: 'global' }, 'only explicit group policy selection checks the parent');
  h.groupSelect().props.onChange('');
  h.nodeSelect().props.onChange('us');
  assert.deepEqual(h.measured[6], { ids: ['us'], onlyStale: true, groupId: undefined }, 'standalone node selection checks one node');
  h.unmount();
});

test('manual group check buttons use the same subtree boundary and can force a fresh check', () => {
  const h = latencyPickerHarness(); h.props.latency.running = false;
  h.props.itemId = 'region'; h.props.selectedGroupId = 'region';
  const tree = h.render();
  const groupCheck = elements(tree).find((entry) => entry.props?.className?.includes('codex-picker-icon-button'));
  groupCheck.props.onClick();
  assert.deepEqual(h.measured[0], { ids: ['us2', 'us'], onlyStale: false, groupId: 'region' });
  h.props.itemId = 'global'; h.props.selectedGroupId = 'global';
  h.policySelect().props.options.find((option: any) => option.value === 'region').measure.run();
  assert.deepEqual(h.measured[1], { ids: ['us2', 'us'], onlyStale: false, groupId: 'region' });
  h.unmount();
});

test('latency views retain node results and cancellation without aggregate progress counts', async () => {
  const i18n = createInstance();
  await i18n.init({ lng: 'en', resources: { en: { translation: en } } });
  for (const running of [true, false]) {
    const currentLatency = { ...latency, running, completed: 6, total: 55,
      results: { us: { status: 'success' as const, value: { latencyMs: 42, checkedAt: Date.now() } } },
    };
    const picker = createElement(CodexProxyPicker, { source, itemId: 'us', selectedGroupId: 'region', selections: {}, busy: false, latency: currentLatency, choose: () => {}, chooseMember: () => {} });
    const nodes = createElement(CodexProxyResourceNodes, { source, itemId: 'us', busy: false, latency: currentLatency, onChoose: () => {}, onInsecure: () => {} });
    for (const view of [picker, nodes]) {
      const html = renderToStaticMarkup(createElement(I18nextProvider, { i18n }, view));
      assert.doesNotMatch(html, /Testing|6\s*\/\s*55/);
      assert.match(html, /42 ms/);
      assert.equal(html.includes('Cancel check'), running);
    }
  }
  const h = latencyPickerHarness();
  const cancel = elements(h.render()).find((entry) => entry.type === 'button' && entry.props.children === 'codex.proxy.cancelCheck');
  cancel.props.onClick(); assert.deepEqual(h.events, ['cancel']);
  h.unmount();
});

test('a child policy waiting for certificate permission can enter the draft without granting permission', async () => {
  const pendingSource: ProxyCatalogSource = { ...source,
    nodes: [{ id: 'tls', name: 'Certificate option node', protocol: 'hysteria2', supported: false, insecure: true, error: 'PROXY_TLS_INSECURE' }],
    groups: [group('region', 'US group', 'select', ['Automatic']), {
      ...group('auto', 'Automatic', 'url-test', ['Certificate option node']), supported: false, error: 'SUBSCRIPTION_GROUP_UNAVAILABLE',
      insecureNodeIds: ['tls'], issues: [{ name: 'Certificate option node', error: 'PROXY_TLS_INSECURE' }],
    }],
  };
  const before = JSON.stringify(pendingSource);
  let catalogChanges = 0;
  const h = latencyPickerHarness();
  h.props.source = pendingSource; h.props.itemId = 'region'; h.props.selectedGroupId = 'region';
  h.props.onCatalogChange = () => { catalogChanges++; };
  const childOption = h.manualSelect().props.options.find((option: any) => option.value === 'Automatic');
  assert.equal(childOption.disabled, false, 'permission-only child groups must remain inspectable');
  h.manualSelect().props.onChange('Automatic');
  assert.equal(h.props.itemId, 'region', 'the manual group remains the bound identity');
  assert.deepEqual(h.props.selections, { region: 'Automatic' });
  assert.equal(selectionUtils.defaultProxySelections(pendingSource, h.props.itemId, h.props.selections), null, 'saving still waits for explicit permission');
  const issue = elements(h.render()).find((entry) => entry.type === CodexProxySelectionIssues);
  assert.equal(issue.props.target.id, 'auto');
  assert.deepEqual(issue.props.target.insecureNodeIds, ['tls']);
  assert.deepEqual(h.measured, [{ ids: [], onlyStale: true, groupId: 'auto' }], 'pending certificate options cannot be silently tested with weakened TLS');
  const html = await markup(pendingSource, 'region', 'region', h.props.selections, { onCatalogChange: h.props.onCatalogChange });
  assert.match(html, /Certificate permission needed/);
  assert.match(html, /Certificate option node/);
  assert.equal(catalogChanges, 0);
  assert.equal(JSON.stringify(pendingSource), before, 'entering a draft never grants or mutates certificate permission');
  h.unmount();
});

const runtimeSource: ProxyCatalogSource = { ...source,
  nodes: [{ ...source.nodes[0], id: 'fast', name: 'Fastest measured node' }, { ...source.nodes[0], id: 'live', name: 'Actual engine node', udp: true }],
  groups: [group('auto', 'Automatic', 'url-test', ['Fastest measured node', 'Actual engine node'])],
};
const runtimeStatus: CodexProxyRuntimeStatus = {
  account: 'running', desktop: 'idle', sidecar: 'unbound', accountPort: 64000, desktopPort: null,
  accountNode: 'Actual engine node', accountSelection: { name: 'Actual engine node', delayMs: 700, checkedAt: 1_600_000_000_000 },
  proxySource: 'account', effectiveProxy: { protocol: 'RESOURCE', sourceId: 'source', itemId: 'auto' },
};
const measuredLatency = { ...latency, results: {
  fast: { status: 'success' as const, value: { latencyMs: 1, checkedAt: 1_600_000_001_000 } },
  live: { status: 'success' as const, value: { latencyMs: 17, checkedAt: 1_600_000_001_000 } },
} };

test('the current node and delay come from the matching live engine rather than the fastest measured candidate', async () => {
  const html = await markup(runtimeSource, 'auto', 'auto', {}, { runtimeStatus, latency: measuredLatency });
  assert.match(html, />Current node<\/span>/);
  assert.match(html, /<strong title="Actual engine node">Actual engine node<\/strong>/);
  assert.match(html, /700 ms/);
  assert.match(html, />UDP<\/span>/);
  assert.doesNotMatch(html, /Fastest measured node|>1 ms<|>17 ms</);
});

test('unmatched or stopped runtimes show the policy draft without inventing a current node from latency', async () => {
  for (const runtime of [null,
    { ...runtimeStatus, effectiveProxy: { ...runtimeStatus.effectiveProxy!, sourceId: 'other-source' } },
    { ...runtimeStatus, effectiveProxy: { ...runtimeStatus.effectiveProxy!, itemId: 'other-item' } },
    { ...runtimeStatus, account: 'stopped' as const },
  ]) {
    const html = await markup(runtimeSource, 'auto', 'auto', {}, { runtimeStatus: runtime, latency: measuredLatency });
    assert.match(html, />Selection<\/span>/);
    assert.match(html, /<strong title="Automatic">Automatic<\/strong>/);
    assert.match(html, /The current node appears when this group is running/);
    assert.doesNotMatch(html, />Current node<\/span>|Fastest measured node|Actual engine node|700 ms|>1 ms<|>17 ms</);
  }
});
