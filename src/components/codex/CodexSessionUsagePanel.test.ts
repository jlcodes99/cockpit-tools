import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { isStandardCodexOAuthAccount } from '../../types/codex';
import type { CodexSessionUsageQuery, CodexSessionUsageReport } from '../../types/codex';
import { buildCodexSessionUsageQuery } from '../../utils/codexSessionUsageQuery';
import * as usageFormat from '../../utils/codexSessionUsageFormat';
import type { CodexSessionUsageRange } from '../../utils/codexStatsRangePreference';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}

function usageReport(count: number): CodexSessionUsageReport {
  return {
    totals: {
      inputTokens: count, cachedInputTokens: 0, outputTokens: 0,
      totalTokens: count, requestCount: count, estimatedCostUsd: 0,
    },
    byModel: [], byDay: [], byInstance: [], byProject: [],
    projectsPending: false, modelOptions: [], projectOptions: [],
    instances: [{ id: 'fixture', name: 'Fixture' }],
    lastSyncedAt: count ? 1_780_000_000 : null,
    filesTracked: count ? 1 : 0,
    eventCount: count, deferredFiles: 0, lastErrorCount: 0,
  };
}

function sessionUsageHarness(invoke: (
  command: string, args: { query: CodexSessionUsageQuery; rebuild?: boolean },
) => Promise<unknown>) {
  const service = loadHookModule(new URL('../../services/codexInstanceService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke },
    '../utils/cliLaunchPath': {},
    '../stores/useCodexPathRecoveryStore': {},
    '../utils/codexProxyEnginePrerequisite': {},
    './platform/createPlatformInstanceService': { createPlatformInstanceService: () => ({}) },
  });
  const t = (key: string) => key;
  return (now = new Date(2026, 10, 19, 10)) => loadHookModule(new URL('./CodexSessionUsagePanel.tsx', import.meta.url), {
    '@tauri-apps/api/event': { listen: async () => () => {} },
    './CodexAccountPlanBadge': { CodexAccountPlanBadge: 'plan-badge' },
    './CodexSessionUsageRequests': {
      CodexSessionUsageRequests: 'usage-requests',
      usageProjectLabel: (path: string) => path.split('/').pop(),
    },
    'react-i18next': { useTranslation: () => ({ t, i18n: { language: 'en' } }) },
    '@tauri-apps/plugin-dialog': { confirm: async () => true },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'usage-filter' },
    '../../services/codexInstanceService': service.exports,
    '../../services/codexService': {},
    '../../stores/useCodexAccountStore': {},
    '../../types/codex': {},
    '../../utils/codexSessionUsageQuery': {
      buildCodexSessionUsageQuery: (range: CodexSessionUsageRange, id: string) =>
        buildCodexSessionUsageQuery(range, id, now),
    },
    '../../utils/codexSessionUsageFormat': usageFormat,
    './CodexUsageTrend': {},
    '../../utils/codexStatsRangePreference': { normalizeCodexSessionUsageRange: () => 'all' },
    '../../hooks/useDurableUiSetting': { useDurableUiSetting: () => 'all' },
    '../../utils/durableUiSettings': {},
    './useCodexProxyExitEditor': {},
  });
}

function detailView(h: ReturnType<ReturnType<typeof sessionUsageHarness>>) {
  const find = (predicate: (node: any) => boolean) => nodes(h.flush()).find(predicate);
  return {
    tokens: () => find((node) => node.props?.className === 'codex-session-usage__metrics')
      ?.props.children[0].props.children[1].props.children.props?.value,
    error: () => find((node) => node.props?.className === 'codex-session-usage__error')?.props.children,
    status: () => find((node) => node.props?.className === 'codex-session-usage__status')?.props.children[0],
    refresh: () => find((node) => node.type === 'button' &&
      node.props.children?.[1] === 'codex.sessionUsage.actions.refresh')?.props,
    selectInstance: () => find((node) => node.type === 'usage-filter' &&
      node.props.ariaLabel === 'codex.sessionUsage.instance.label')?.props.onChange('fixture'),
  };
}

test('usage detail shows its cache and shares the list scan when opened during scanning', async () => {
  const scan = deferred<{ report: CodexSessionUsageReport; errors: string[] }>();
  const cached = deferred<CodexSessionUsageReport>();
  const detailOpenedAt = new Date(2026, 10, 19, 10, 1);
  let reads = 0;
  let scans = 0;
  const createHarness = sessionUsageHarness(async (command, { query }) => {
    if (command === 'codex_query_session_usage') {
      if (++reads === 1) return usageReport(4);
      assert.equal(query.toTimestamp, detailOpenedAt.getTime() / 1_000);
      return reads === 2 ? cached.promise : usageReport(8);
    }
    assert.equal(command, 'codex_sync_session_usage');
    if (++scans > 1) throw '数据正在处理中，请稍后重试。';
    return scan.promise;
  });
  const summary = createHarness();
  summary.render(() => summary.exports.CodexSessionUsageSummary({ onOpenDetail() {} }));
  await settlePromises();
  assert.equal(scans, 1);
  summary.unmount();

  const detail = createHarness(detailOpenedAt);
  detail.render(() => detail.exports.CodexSessionUsagePanel());
  const view = detailView(detail);
  cached.resolve(usageReport(4));
  await settlePromises();
  assert.equal(view.error(), undefined);
  assert.equal(view.status(), 'codex.sessionUsage.status.scanning');
  assert.equal(view.tokens(), 4);
  assert.equal(scans, 1);

  scan.resolve({ report: usageReport(7), errors: [] });
  await settlePromises();
  assert.equal(view.tokens(), 8);
  assert.equal(view.refresh().disabled, false);
  detail.unmount();
});

test('changing the instance during a scan shows the new filter after completion', async () => {
  const scan = deferred<{ report: CodexSessionUsageReport; errors: string[] }>();
  let refreshed = false;
  let scans = 0;
  const detail = sessionUsageHarness(async (command, { query }) => {
    if (command === 'codex_query_session_usage') {
      return usageReport(query.instanceId === 'fixture' ? (refreshed ? 9 : 2) : 4);
    }
    assert.equal(command, 'codex_sync_session_usage');
    scans++;
    return scan.promise;
  })();
  detail.render(() => detail.exports.CodexSessionUsagePanel());
  const view = detailView(detail);
  view.selectInstance();
  detail.flush();
  await settlePromises();
  assert.equal(view.tokens(), 2);
  assert.equal(view.refresh().disabled, true);

  refreshed = true;
  scan.resolve({ report: usageReport(15), errors: [] });
  await settlePromises();
  assert.equal(view.tokens(), 9);
  assert.equal(view.refresh().disabled, false);
  assert.equal(scans, 1);
  detail.unmount();
});

test('a failed scan releases refresh and keeps cached totals visible', async () => {
  let scans = 0;
  const detail = sessionUsageHarness(async (command) => {
    if (command === 'codex_query_session_usage') return usageReport(4);
    assert.equal(command, 'codex_sync_session_usage');
    if (++scans === 1) throw new Error('fixture scan failed');
    return { report: usageReport(8), errors: [] };
  })();
  detail.render(() => detail.exports.CodexSessionUsagePanel());
  const view = detailView(detail);
  await settlePromises();
  assert.equal(view.tokens(), 4);
  assert.equal(view.error(), 'codex.sessionUsage.errors.syncFailed');
  assert.equal(view.refresh().disabled, false);

  view.refresh().onClick();
  await settlePromises();
  assert.equal(view.tokens(), 8);
  assert.equal(view.error(), undefined);
  assert.equal(scans, 2);
  detail.unmount();
});

test('official lifetime usage follows the chosen OAuth account and distinguishes zero from missing', async () => {
  const requests: string[] = [];
  const pending = deferred<number | null>();
  const accounts = [
    { id: 'a', auth_mode: 'oauth', tokens: { access_token: 'fixture-a', refresh_token: 'fixture-a' } },
    { id: 'b', auth_mode: 'oauth', tokens: { access_token: 'fixture-b', refresh_token: 'fixture-b' } },
    { id: 'api', auth_mode: 'apikey', tokens: { access_token: 'api-key' } },
  ];
  let nextValue: number | null = null;
  let shouldFail = false;
  const h = loadHookModule(new URL('./CodexSessionUsagePanel.tsx', import.meta.url), {
    '@tauri-apps/api/event': { listen: async () => () => {} },
    './CodexAccountPlanBadge': { CodexAccountPlanBadge: 'plan-badge' },
    './CodexSessionUsageRequests': {
      CodexSessionUsageRequests: 'usage-requests',
      usageProjectLabel: (path: string) => path.split('/').pop(),
    },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '@tauri-apps/plugin-dialog': {},
    '../SingleSelectDropdown': { SingleSelectDropdown: 'account-picker' },
    '../../services/codexInstanceService': {},
    '../../services/codexService': { getCodexProfileLifetimeTokens: async (accountId: string) => {
      requests.push(accountId);
      if (accountId === 'a') return pending.promise;
      if (shouldFail) throw new Error('upstream rejected the lookup');
      return nextValue;
    } },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (select: any) => select({ accounts, currentAccount: accounts[0] }) },
    '../../types/codex': { isStandardCodexOAuthAccount },
    '../../utils/codexSessionUsageQuery': {},
    '../../utils/codexSessionUsageFormat': usageFormat,
    './CodexUsageTrend': {},
    '../../utils/codexStatsRangePreference': {},
    '../../hooks/useDurableUiSetting': {},
    '../../utils/durableUiSettings': {},
    './useCodexProxyExitEditor': { useCodexProxyAccountName: () => (account: { id: string }) => `masked ${account.id}` },
  });
  h.render(() => h.exports.CodexProfileUsageTotal({ lang: 'en' }));
  const find = (predicate: (node: any) => boolean) => nodes(h.flush()).find(predicate);
  const status = () => find((node) => node.props?.role === 'status')?.props.children;
  const refresh = () => find((node) => node.type === 'button').props.onClick();
  const picker = () => find((node) => node.type === 'account-picker');
  assert.deepEqual(Array.from(picker().props.options, (option: any) => option.value), ['a', 'b']);
  assert.equal(picker().props.value, 'a');
  assert.deepEqual(requests, []);

  refresh();
  assert.equal(status(), 'common.loading');
  picker().props.onChange('b');
  refresh();
  await settlePromises();
  assert.equal(status(), 'codex.sessionUsage.comparison.unavailable');

  pending.resolve(630_000_000);
  await settlePromises();
  assert.equal(status(), 'codex.sessionUsage.comparison.unavailable');
  nextValue = 0;
  refresh();
  await settlePromises();
  assert.equal(status().props.value, 0);

  shouldFail = true;
  refresh();
  await settlePromises();
  assert.equal(status(), 'codex.sessionUsage.comparison.loadError');
  assert.deepEqual(requests, ['a', 'b', 'b', 'b']);
  h.unmount();
});

test('project drill-down preserves the model filter and shares the exact session scope', async () => {
  const project = {
    key: '/work/same-name', label: '/work/same-name',
    inputTokens: 5, cachedInputTokens: 3, outputTokens: 1, totalTokens: 6, requestCount: 1,
    estimatedCostUsd: 0.1,
  };
  const detail = sessionUsageHarness(async (command, { query }) => {
    const scopedReport = {
      ...usageReport(query.sessionId ? 1 : 6),
      byProject: [project],
      projectOptions: [{ id: project.key, name: project.label }],
      modelOptions: [{ id: 'gpt-5.4', name: 'gpt-5.4' }],
    };
    return command === 'codex_sync_session_usage' ? { report: scopedReport, errors: [] } : scopedReport;
  })();
  detail.render(() => detail.exports.CodexSessionUsagePanel());
  await settlePromises();
  const find = (predicate: (node: any) => boolean) => nodes(detail.flush()).find(predicate);
  find((node) => node.type === 'usage-filter'
    && node.props.ariaLabel === 'codex.sessionUsage.tables.modelName').props.onChange('gpt-5.4');
  find((node) => node.type === 'button'
    && node.props.children === 'codex.sessionUsage.analysis.project').props.onClick();
  detail.flush();
  await settlePromises();

  const breakdown = find((node) => typeof node.props?.onSelect === 'function');
  assert.equal(breakdown.props.rows[0].label, 'same-name');
  breakdown.props.onSelect(breakdown.props.rows[0]);
  const requests = () => find((node) => node.type === 'usage-requests').props;
  assert.equal(requests().query.projectKey, project.key);
  assert.equal(requests().query.model, 'gpt-5.4');
  requests().onSelectSession('session-id', 'Example session');
  assert.equal(requests().query.sessionId, 'session-id');
  find((node) => node.props?.['aria-label'] === 'codex.sessionUsage.analysis.clearSession').props.onClick();
  assert.equal(requests().query.sessionId, undefined);
  assert.equal(requests().query.projectKey, project.key);
  detail.unmount();
});

test('request pages keep the cursor and show unknown prices without a zero charge', async () => {
  const row = {
    requestId: 'first', timestamp: 1_780_000_000, instanceId: 'fixture',
    instanceName: 'Fixture', sessionId: 'thread', sessionTitle: 'Conversation',
    projectPath: '/work/project', model: 'unpriced', inputTokens: 20,
    cachedInputTokens: 15, outputTokens: 4, estimatedCostUsd: null,
    firstResponseMs: 520, tokensPerSecond: 64.5,
  };
  const next = { timestamp: row.timestamp, requestId: row.requestId };
  const reads: unknown[] = [];
  const panel = loadHookModule(new URL('./CodexSessionUsageRequests.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexSessionUsageFormat': usageFormat,
    '../../services/codexInstanceService': {
      querySessionUsageRequests: async (_query: unknown, cursor: unknown) => {
        reads.push(cursor);
        return cursor
          ? { rows: [{ ...row, requestId: 'second' }], nextCursor: null }
          : { rows: [row], nextCursor: next };
      },
    },
  });
  const query = { projectKey: '/work/project' };
  const selected: string[] = [];
  panel.render(() => panel.exports.CodexSessionUsageRequests({
    query, revision: null, lang: 'en',
    onSelectSession: (id: string) => selected.push(id),
  }));
  await settlePromises();
  const find = (predicate: (node: any) => boolean) => nodes(panel.flush()).find(predicate);
  const click = (label: string) => find((node) => node.type === 'button' && node.props.children === label).props.onClick();
  assert.equal(find((node) => node.type === 'td'
    && node.props.className === 'codex-session-usage__col-cost').props.children, '—');
  assert.equal(find((node) => node.type === 'td'
    && node.props.className === 'codex-session-usage__col-firstResponse').props.children, '520 ms');
  assert.equal(find((node) => node.type === 'td'
    && node.props.className === 'codex-session-usage__col-tps').props.children, '64.5 tok/s');
  find((node) => node.type === 'button' && node.props['aria-expanded'] === false).props.onClick();
  click('codex.sessionUsage.analysis.viewSession');
  assert.deepEqual(selected, ['thread']);
  click('pagination.next');
  panel.flush();
  await settlePromises();
  assert.equal(find((node) => node.type === 'button' && node.props.children === 'pagination.next').props.disabled, true);
  click('pagination.prev');
  panel.flush();
  await settlePromises();
  assert.deepEqual(reads, [null, next, null]);
  panel.unmount();
});
