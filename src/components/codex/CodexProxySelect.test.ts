import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';
import { CodexProxySelect, orderProxySelectOptions, type CodexProxySelectProps, type ProxySelectOption } from './CodexProxySelect';
import { CodexProxyLatencyBadge, proxyLatencyTone } from './CodexProxyLatencyBadge';
import type { LatencyState } from '../../utils/codexProxyLatency';
import en from '../../locales/en-US.json';

const options: ProxySelectOption[] = [
  { value: 'policy', label: 'Group policy', pinned: true },
  { value: 'unknown', label: 'Unmeasured' },
  { value: 'fast', label: 'US node 2', delay: 100 },
  { value: 'slow', label: 'US node 10', delay: 550 },
  { value: 'equal', label: 'Another node', delay: 100 },
];
const ids = (data: ProxySelectOption[]) => data.map((entry) => entry.value);
const measured = (ms: number): LatencyState => ({ status: 'success', value: { latencyMs: ms, checkedAt: 1_600_000_000_000 } });

function elements(tree: any): any[] {
  if (!tree || typeof tree !== 'object') return [];
  return Array.isArray(tree) ? tree.flatMap(elements) : [tree, ...elements(tree.props?.children)];
}

function controlHarness(initial: Partial<CodexProxySelectProps> = {}) {
  let topmost: (() => void) | undefined;
  let measuredCount = 0;
  const changes: string[] = [];
  const h = loadHookModule(new URL('./CodexProxySelect.tsx', import.meta.url), {
    'react-dom': { createPortal: (content: unknown) => content },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void) => { topmost = open ? close : undefined; } },
  }, {
    Node: class {},
    document: { body: {}, addEventListener() {}, removeEventListener() {} },
    window: { innerHeight: 600, innerWidth: 800, addEventListener() {}, removeEventListener() {} },
  });
  const props: CodexProxySelectProps = {
    value: 'fast', options: options.map((option) => ({ ...option,
      measure: { label: 'Test', disabled: false, run: () => measuredCount++ },
    })), label: 'Nodes', placeholder: 'Select a node', sortable: true,
    onChange: (value) => changes.push(value), ...initial,
  };
  let tree = h.render(() => h.exports.CodexProxySelect(props));
  const render = () => { tree = h.flush(); return tree; };
  const all = () => elements(tree);
  const trigger = () => all().find((element) => element.props?.className === 'codex-proxy-select-trigger');
  const menu = () => all().find((element) => element.props?.role === 'dialog');
  return { props, h, render, all, trigger, menu, changes,
    open() { trigger().props.onClick(); render(); },
    sort(kind: string) { all().find((element) => element.props?.className === 'codex-proxy-select-sort' && element.props.children.endsWith(`_${kind}`)).props.onClick(); render(); },
    optionIds() { return all().filter((element) => element.props?.role === 'option').map((element) => element.key || element.props.title); },
    optionLabels() { return all().filter((element) => element.props?.role === 'option').map((element) => String(element.props.title).split('\n')[0]); },
    get measuredCount() { return measuredCount; },
    get topmost() { return topmost; },
  };
}

test('node sorting keeps policies first and equal or unknown delays stable', () => {
  assert.deepEqual(ids(orderProxySelectOptions(options, 'default')), ['policy', 'unknown', 'fast', 'slow', 'equal']);
  assert.deepEqual(ids(orderProxySelectOptions(options, 'latency')), ['policy', 'fast', 'equal', 'slow', 'unknown']);
  assert.deepEqual(ids(orderProxySelectOptions(options, 'name')), ['policy', 'equal', 'unknown', 'fast', 'slow']);
  const invalid = [{ value: 'a', label: 'A', delay: -1 }, { value: 'b', label: 'B', delay: NaN }, { value: 'c', label: 'C', delay: Infinity }];
  assert.deepEqual(ids(orderProxySelectOptions(invalid, 'latency')), ['a', 'b', 'c']);
  assert.deepEqual(ids(options), ['policy', 'unknown', 'fast', 'slow', 'equal'], 'sorting does not mutate callers');
});

test('a frozen batch order keeps current rows and appends new rows without reusing stale options', () => {
  const updated = [options[0], { ...options[2], delay: 900 }, { ...options[3], delay: 1 }, { value: 'new', label: 'New', delay: 2 }];
  const frozen = orderProxySelectOptions(updated, 'latency', ['policy', 'fast', 'slow', 'unknown']);
  assert.deepEqual(ids(frozen), ['policy', 'fast', 'slow', 'new']);
  assert.equal(frozen[1].delay, 900, 'the badge receives the latest value while position remains stable');
});

test('mounting, opening, searching and sorting never measure a node or change the selection', () => {
  const h = controlHarness();
  assert.equal(h.menu(), undefined);
  h.open();
  h.sort('latency');
  const input = h.all().find((element) => element.type === 'input');
  input.props.onChange({ target: { value: 'uS NODE' } }); h.render();
  assert.deepEqual(h.optionLabels(), ['US node 2', 'US node 10']);
  assert.equal(h.measuredCount, 0);
  assert.deepEqual(h.changes, []);
  assert.equal(h.props.value, 'fast');
  h.h.unmount();
});

test('latency changes update in place while testing, then reorder after the batch finishes', () => {
  const h = controlHarness();
  h.open(); h.sort('latency');
  assert.deepEqual(h.optionLabels(), ['Group policy', 'US node 2', 'Another node', 'US node 10', 'Unmeasured']);
  h.props.measuring = true;
  h.props.options = h.props.options.map((option) => ({ ...option, delay: option.value === 'slow' ? 1 : option.value === 'fast' ? 900 : option.delay }));
  h.render();
  assert.deepEqual(h.optionLabels(), ['Group policy', 'US node 2', 'Another node', 'US node 10', 'Unmeasured']);
  h.props.measuring = false; h.render();
  assert.deepEqual(h.optionLabels(), ['Group policy', 'US node 10', 'Another node', 'US node 2', 'Unmeasured']);
  assert.equal(h.props.value, 'fast');
  assert.deepEqual(h.changes, []);
  h.h.unmount();
});

test('explicit node tests leave the menu and selected value intact; choosing a node closes only the menu', () => {
  const h = controlHarness(); h.open();
  const button = h.all().find((element) => element.props?.['aria-label'] === 'US node 10 · Test');
  button.props.onClick(); h.render();
  assert.equal(h.measuredCount, 1);
  assert.ok(h.menu());
  assert.deepEqual(h.changes, []);
  h.all().find((element) => element.props?.role === 'option' && element.props.title === 'US node 10').props.onClick(); h.render();
  assert.deepEqual(h.changes, ['slow']);
  assert.equal(h.menu(), undefined);
  h.h.unmount();
});

test('Escape registers as the topmost layer and consumes a menu key event without changing a draft', () => {
  const h = controlHarness(); h.open();
  assert.equal(typeof h.topmost, 'function');
  let prevented = false; let stopped = false;
  h.menu().props.onKeyDown({ key: 'Escape', preventDefault() { prevented = true; }, stopPropagation() { stopped = true; } }); h.render();
  assert.equal(prevented, true); assert.equal(stopped, true);
  assert.equal(h.menu(), undefined);
  assert.equal(h.topmost, undefined);
  assert.deepEqual(h.changes, []);
  h.open(); h.topmost!(); h.render();
  assert.equal(h.menu(), undefined);
  assert.deepEqual(h.changes, []);
  h.h.unmount();
});

test('unavailable entries cannot change a selection and disabling the control closes its menu', () => {
  const h = controlHarness({ options: [{ value: 'bad', label: 'Bad', disabled: true }] });
  h.open();
  const option = h.all().find((element) => element.props?.role === 'option');
  option.props.onClick(); h.render();
  assert.deepEqual(h.changes, []);
  h.props.disabled = true; h.render();
  assert.equal(h.menu(), undefined);
  assert.equal(h.trigger().props.disabled, true);
  h.h.unmount();
});

test('delay badge colors distinguish native measurements from pending and failed checks', () => {
  assert.equal(proxyLatencyTone(undefined), 'muted');
  assert.equal(proxyLatencyTone({ status: 'cancelled' }), 'muted');
  assert.equal(proxyLatencyTone({ status: 'queued' }), 'loading');
  assert.equal(proxyLatencyTone({ status: 'running' }), 'loading');
  assert.equal(proxyLatencyTone(measured(249)), 'good');
  assert.equal(proxyLatencyTone(measured(250)), 'medium');
  assert.equal(proxyLatencyTone(measured(399)), 'medium');
  assert.equal(proxyLatencyTone(measured(400)), 'slow');
  assert.equal(proxyLatencyTone(measured(NaN)), 'error');
  assert.equal(proxyLatencyTone({ status: 'error', error: 'TIMEOUT' }), 'error');
});

test('static trigger and delay badge show the selected name, native delay and sanitized error details', async () => {
  const i18n = createInstance();
  await i18n.init({ lng: 'en', resources: { en: { translation: en } } });
  const render = (child: ReturnType<typeof createElement>) => renderToStaticMarkup(createElement(I18nextProvider, { i18n }, child));
  const badge = render(createElement(CodexProxyLatencyBadge, { result: measured(216) }));
  assert.match(badge, /is-good/); assert.match(badge, /216 ms/); assert.match(badge, /Checked at/);
  assert.doesNotMatch(badge, /HTTP|HTTPS/);
  const pending = render(createElement(CodexProxyLatencyBadge));
  assert.match(pending, /Not tested/); assert.match(pending, />-<\/span>/);
  const loading = render(createElement(CodexProxyLatencyBadge, { result: { status: 'running' } }));
  assert.match(loading, /Testing/); assert.match(loading, /codex-proxy-latency-spinner/);
  const error = render(createElement(CodexProxyLatencyBadge, { result: { status: 'error', error: 'https://private.invalid/?token=secret' } }));
  assert.doesNotMatch(error, /private\.invalid|token=secret/);
  const select = render(createElement(CodexProxySelect, { value: 'us', options: [{ value: 'us', label: 'US node', badge: createElement(CodexProxyLatencyBadge, { result: measured(216) }) }],
    label: 'Node', placeholder: 'Select', onChange() {} }));
  assert.match(select, /US node/); assert.match(select, /216 ms/); assert.match(select, /aria-expanded="false"/);
  assert.doesNotMatch(select, /<select|role="dialog"/);
});
