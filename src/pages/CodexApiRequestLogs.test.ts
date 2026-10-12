import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { resolveCodexApiServiceLogModelPair } from '../utils/codexApiServiceLogModel';
import { hasRequestFirstResponse, requestServiceTierPresentation, requestedServiceTierLabelKey } from '../utils/codexRequestDiagnostics';

type Element = { type: unknown; props: Record<string, any> };
function nodes(value: any): Element[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [{ type: value.type, props: value.props ?? {} }, ...nodes(value.props?.children)];
}
function text(value: any): string {
  if (value == null || typeof value === 'boolean') return '';
  if (Array.isArray(value)) return value.map(text).join('');
  return typeof value === 'object' ? text(value.props?.children) : String(value);
}
const sample = {
  timestamp: 123, requestId: 'request-1', accountId: 'account-1', email: 'private@example.com',
  apiKeyId: 'key-1', apiKeyLabel: 'My key', clientInstanceId: 'instance-1',
  modelId: 'gpt-model', upstreamModel: 'gpt-model', requestKind: 'text', reasoningEffort: 'medium',
  serviceTier: 'standard', success: true, httpStatus: 200, errorCategory: '', errorMessage: '',
  latencyMs: 9000, firstResponseMs: 2000, inputTokens: 1500, outputTokens: 300, cachedTokens: 1000,
  totalTokens: 1800, estimatedCostUsd: 0.013, proxyRoute: { kind: 'direct' },
};
function props(events: any[] = [sample]) {
  return {
    t: (key: string, values?: any) => values?.status ? `${key}:${values.status}` : values?.tier ? `${key}:${values.tier}` : key,
    accountDisplayNames: new Map(), codexInstances: [], requestLogEvents: events, requestLogLoaded: true,
    cleanRequestLogErrorDetail: (value: string) => value,
    truncateRequestLogErrorDetail: (value: string) => value.slice(0, 100),
    maskAccountText: (value: string) => value.replace(/[^@]/g, '*'),
    resolveClientInstanceLabel: () => 'My instance', requestKindLabel: () => 'text',
    formatCompactNumber: String, formatDateTime: () => '10/10 17:05:03', formatLatencyMs: (v: number) => `${v}ms`,
    formatUsdCost: (v: number) => `$${v}`, requestLogInstanceOptions: [], requestLogKindOptions: [], requestLogStatusOptions: [],
    REQUEST_LOG_PAGE_SIZE_OPTIONS: [20], requestLogCurrentPage: 1, requestLogTotalPages: 1, requestLogTotal: events.length,
  };
}
function harness(localStorage: any) {
  return loadHookModule(new URL('./CodexApiRequestLogs.tsx', import.meta.url), {
    '../components/codex/CodexOutputSpeed': { CodexOutputSpeed: 'CodexOutputSpeed' },
    '../utils/codexOutputSpeed': { aggregateOutputSpeed: () => ({ tokensPerSecond: null, samples: 0 }), formatOutputSpeed: () => null },
    'lucide-react': { Check: 'Check', CircleAlert: 'CircleAlert', Columns3: 'Columns3', Zap: 'Zap' },
    '../components/SingleSelectDropdown': { SingleSelectDropdown: 'SingleSelectDropdown' },
    '../components/PaginationControls': { PaginationControls: 'PaginationControls' },
    '../components/codex/CodexRequestProxyLabel': { CodexRequestProxyLabel: 'CodexRequestProxyLabel' },
    '../components/codex/CodexRequestDetailModal': { CodexRequestDetailModal: 'CodexRequestDetailModal' },
    '../utils/codexRequestDiagnostics': { hasRequestFirstResponse, requestServiceTierPresentation, requestedServiceTierLabelKey },
    '../utils/codexApiServiceLogModel': { resolveCodexApiServiceLogModelPair },
  }, { localStorage });
}
function storage(initial: string | null = null) {
  let value = initial;
  return { getItem: () => value, setItem: (_key: string, next: string) => { value = next; } };
}
function openColumns(hook: ReturnType<typeof harness>) {
  const button = nodes(hook.flush()).find((n) => n.type === 'button' && n.props['aria-haspopup'])!;
  button.props.onClick();
  return nodes(hook.flush()).filter((n) => n.type === 'button' && 'aria-pressed' in n.props);
}

test('speed badges use short labels and retain raw tier evidence only in details', () => {
  const hook = harness(storage());
  hook.render(() => hook.exports.CodexApiRequestLogs(props([
    { ...sample, requestedServiceTier: 'priority', responseServiceTier: 'default' },
    { ...sample, requestId: 'legacy' },
    { ...sample, requestId: 'automatic', requestedServiceTier: 'auto', responseServiceTier: 'default' },
    { ...sample, requestId: 'standard', requestedServiceTier: 'default', responseServiceTier: 'default' },
    { ...sample, requestId: 'fast-alias', requestedServiceTier: 'FAST', responseServiceTier: 'default' },
    { ...sample, requestId: 'response-only', responseServiceTier: 'priority' },
  ])));
  const rows = nodes(hook.flush()).filter((n) => n.type === 'tr').slice(1);
  const badges = rows.map((row) => nodes(row).filter((n) => n.props.className?.startsWith('codex-request-log-speed')));
  assert.deepEqual(badges.map((items) => items.map(text)), [
    ['codex.speed.fast'], [], [], ['codex.speed.standard'], ['codex.speed.fast'], [],
  ]);
  for (const row of rows) assert.doesNotMatch(text(row), /requestedTier|responseTier|legacyTier|default/);
  for (const index of [0, 4]) {
    assert.equal(badges[index][0].type, 'span');
    assert.match(badges[index][0].props.className, / fast$/);
    assert.equal(badges[index][0].props.onClick, undefined);
    assert.equal(nodes(badges[index][0]).filter((n) => n.type === 'Zap').length, 1);
  }
  assert.equal(nodes(badges[3][0]).filter((n) => n.type === 'Zap').length, 0);
  const details = nodes(rows[0]).find((n) => n.type === 'button' && text(n) === 'codex.requestDiagnostics.details')!;
  details.props.onClick();
  const event = nodes(hook.flush()).find((n) => n.type === 'CodexRequestDetailModal')!.props.event;
  assert.equal(event.requestedServiceTier, 'priority');
  assert.equal(event.responseServiceTier, 'default');
});

test('fixed log columns preserve source labels, actual model, privacy, token breakdown and detail entry', () => {
  const hook = harness(storage());
  hook.render(() => hook.exports.CodexApiRequestLogs(props([sample, { ...sample, requestId: '', upstreamModel: '', success: false, httpStatus: 500, errorCategory: 'upstream_error', errorMessage: 'Full error detail' }])));
  const tree = nodes(hook.flush());
  const headers = tree.filter((n) => n.type === 'th').map((n) => text(n));
  assert.deepEqual(headers.map((v) => v.split('.').pop()), ['time', 'model', 'result', 'source', 'account', 'tokens', 'cost', 'performance', 'actions']);
  const rows = tree.filter((n) => n.type === 'tr').slice(1);
  for (const row of rows) assert.equal(nodes(row).filter((n) => n.type === 'td').length, headers.length);
  assert.equal(tree.filter((n) => n.props.title === 'codex.apiService.logs.upstreamModel: gpt-model').length, 1);
  assert.match(text(rows[0]), /apiKeyFilter\s+My key/);
  assert.match(text(rows[0]), /instanceFilter\s+My instance/);
  assert.match(text(rows[0]), /tokenInput1500.*tokenOutput300.*tokenCache1000.*tokenTotal1800/);
  assert.equal(text(rows[0]).includes(sample.email), false);
  assert.equal(tree.some((n) => n.props.title === sample.email), false);
  assert.ok(tree.some((n) => n.props.title === 'Full error detail' && text(n) === 'upstream_error'));
  const details = tree.filter((n) => n.type === 'button' && text(n) === 'codex.requestDiagnostics.details');
  assert.equal(details.length, 1);
  details[0].props.onClick();
  assert.equal(nodes(hook.flush()).find((n) => n.type === 'CodexRequestDetailModal')!.props.event, sample);
});

test('column choices persist and prevent hiding both model and result', () => {
  const saved = storage();
  const hook = harness(saved);
  hook.render(() => hook.exports.CodexApiRequestLogs(props()));
  let options = openColumns(hook);
  options.find((n) => text(n).endsWith('.proxy'))!.props.onClick();
  assert.ok(JSON.parse(saved.getItem()!).includes('proxy'));
  options.find((n) => text(n).endsWith('.result'))!.props.onClick();
  options = nodes(hook.flush()).filter((n) => n.type === 'button' && 'aria-pressed' in n.props);
  assert.equal(options.find((n) => text(n).endsWith('.model'))!.props.disabled, true);
  const restored = harness(saved);
  restored.render(() => restored.exports.CodexApiRequestLogs(props()));
  const headers = nodes(restored.flush()).filter((n) => n.type === 'th').map(text);
  assert.ok(headers.includes('codex.apiService.logs.columns.proxy'));
  assert.equal(headers.includes('codex.apiService.logs.columns.result'), false);
  assert.ok(headers.includes('codex.apiService.logs.columns.model'));
});

test('damaged or unavailable preferences degrade to usable columns and existing rows survive refresh', () => {
  for (const saved of [storage('malformed'), { getItem() { throw Error('disabled'); }, setItem() { throw Error('disabled'); } }, storage('["unknown", "tokens"]')]) {
    const hook = harness(saved);
    hook.render(() => hook.exports.CodexApiRequestLogs({ ...props(), requestLogLoading: true }));
    let tree = nodes(hook.flush());
    assert.ok(tree.some((n) => n.type === 'th' && text(n) === 'codex.apiService.logs.columns.model'));
    assert.equal(tree.filter((n) => n.type === 'table').length, 1);
    assert.equal(text(hook.flush()).includes('codex.apiService.logs.loading'), false);
    assert.equal(tree.find((n) => n.props.role === 'region')!.props['aria-busy'], true);
    const options = openColumns(hook);
    options.find((n) => text(n).endsWith('.proxy'))!.props.onClick();
    tree = nodes(hook.flush());
    assert.ok(tree.some((n) => n.type === 'th' && text(n) === 'codex.apiService.logs.columns.proxy'));
  }
  const initial = harness(storage());
  initial.render(() => initial.exports.CodexApiRequestLogs({ ...props([]), requestLogLoaded: false, requestLogLoading: true }));
  assert.ok(text(initial.flush()).includes('codex.apiService.logs.loading'));
  const empty = harness(storage());
  empty.render(() => empty.exports.CodexApiRequestLogs({ ...props([]), requestLogLoading: true }));
  assert.equal(text(empty.flush()).includes('codex.apiService.logs.loading'), false);
  assert.ok(text(empty.flush()).includes('codex.localAccess.requestLogEmpty'));
  const failed = harness(storage());
  failed.render(() => failed.exports.CodexApiRequestLogs({ ...props(), requestLogError: 'Refresh failed' }));
  assert.ok(text(failed.flush()).includes('Refresh failed'));
  assert.equal(nodes(failed.flush()).filter((n) => n.type === 'table').length, 1);
  const css = readFileSync(new URL('./CodexApiRequestLogs.css', import.meta.url), 'utf8');
  assert.match(css, /\.codex-request-log-scroll\s*\{[^}]*overflow-x: auto/s);
  assert.match(css, /\.codex-request-log-table\s*\{[^}]*table-layout: fixed/s);
  assert.match(css, /\.codex-request-log-token-grid\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\)/s);
  assert.match(css, /\.codex-request-log-column-options\s*\{[^}]*max-height: min\(420px, 60vh\)/s);
});

test('timeout navigation closes request details and reopening retains the selected record', () => {
  const hook = harness(storage());
  const targets: string[] = [];
  hook.render(() => hook.exports.CodexApiRequestLogs({ ...props(), openTimeoutSettings: (field: string) => targets.push(field) }));
  const open = () => nodes(hook.flush()).find((n) => n.type === 'button' && text(n) === 'codex.requestDiagnostics.details')!.props.onClick();
  open();
  nodes(hook.flush()).find((n) => n.type === 'CodexRequestDetailModal')!.props.onOpenTimeoutSettings('sidecarStreamOpenTimeoutMs');
  assert.deepEqual(targets, ['sidecarStreamOpenTimeoutMs']);
  assert.equal(nodes(hook.flush()).some((n) => n.type === 'CodexRequestDetailModal'), false);
  open();
  assert.equal(nodes(hook.flush()).find((n) => n.type === 'CodexRequestDetailModal')!.props.event, sample);
  hook.unmount();
});
