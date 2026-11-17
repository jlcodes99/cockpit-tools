import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

function importDialog(tab: 'token' | 'paste', content: string) {
  const imports: Record<string, unknown> = {};
  for (const name of [
    '../LaunchPathSettingsButton', '../TagEditModal', '../ExportJsonModal',
    '../ModalErrorMessage', '../MfaQuickCodeSelect', '../QuickSettingsPopover',
    '../PaginationControls', '../AccountSelectionToolbar', '../platform/DosageNotifyUsageStatus',
    '../codebuddy/CodeBuddyQuotaCategoryList', '../AccountLastUsed', '../MultiSelectFilterDropdown',
  ]) imports[name] = {};
  Object.assign(imports, {
    'react-dom': { createPortal: (children: unknown) => children },
    '../../hooks/useAccountsOverviewFilterState': { useAccountsOverviewFilterState: () => [[], () => {}] },
    '../../hooks/usePlatformAccountGroups': { usePlatformAccountGroups: () => ({ groups: [], filterAccountsByGroup: (a: unknown) => a }) },
    '../../hooks/useCodebuddySuitePage': { useCodebuddySuitePage: () => ({
      tierSummary: { all: 0, validCount: 0, dynamicCounts: new Map(), extraKeys: [] },
      filteredAccounts: [], filteredIds: [], groupedAccounts: [],
    }) },
    '../../hooks/usePagination': {
      usePagination: () => ({ pageItems: [] }), buildPaginatedGroups: () => [],
      buildPaginationPageSizeStorageKey: () => 'fixture', isEveryIdSelected: () => false,
    },
    '../../hooks/useEscClose': { useEscClose() {} },
    '../../hooks/useEnterConfirm': { useEnterConfirm() {} },
    '../../stores/useUiConfigStore': { useUiConfigStore: () => 'last_used' },
  });
  const h = loadHookModule(new URL('./CodebuddySuiteAccountsSharedView.tsx', import.meta.url), imports, { document: { body: {} } });
  const requests: { content: string; result: ReturnType<typeof deferred<void>> }[] = [];
  h.render(() => {
    const [tokenInput, setTokenInput] = h.hooks.useState(content);
    const [importing, setImporting] = h.hooks.useState(false);
    const [showAddModal, setShowAddModal] = h.hooks.useState(true);
    const [addStatus, setAddStatus] = h.hooks.useState('idle');
    const page = {
      t: (key: string, fallback?: any) => typeof fallback === 'string' ? fallback : fallback?.defaultValue ?? key,
      locale: 'en', selected: new Set(), tagFilter: [], availableTags: [],
      searchQuery: '', getScopedSelectedCount: () => 0, addTab: tab,
      showAddModal, tokenInput, setTokenInput, importing, addStatus,
      handleTokenImport: async () => {
        const request = { content: tokenInput, result: deferred<void>() };
        requests.push(request);
        setImporting(true);
        try {
          await request.result.promise;
          setShowAddModal(false);
        } catch {
          setAddStatus('error');
        } finally {
          setImporting(false);
        }
      },
    };
    return h.exports.CodebuddySuiteAccountsSharedView({
      accounts: [], loading: false, page,
      platformConfig: { pageClassName: 'workbuddy-accounts-page', showPasteJsonTab: true, flowNotice: {} },
      onRefreshAccounts() {},
    });
  });
  const textarea = () => nodes(h.flush()).find(n => n.type === 'textarea' && n.props.className === 'token-input');
  const submit = () => nodes(h.flush()).find(n => n.type === 'button' && n.props.className === 'btn btn-primary btn-full' && n.props.onClick)?.props.onClick();
  return { h, requests, textarea, submit };
}

for (const tab of ['token', 'paste'] as const) {
  test(`${tab} import releases the large rendered value while retaining the full payload for retry`, async () => {
    const content = JSON.stringify({ quota_raw: 'quota'.repeat(100000), usage_raw: 'usage'.repeat(100000) });
    const dialog = importDialog(tab, content);
    assert.equal(dialog.textarea().props.value, content);
    const first = dialog.submit();
    assert.equal(dialog.requests[0].content, content);
    assert.equal(dialog.textarea().props.value.length, 0);
    assert.equal(dialog.textarea().props.readOnly, true);

    dialog.requests[0].result.reject(new Error('Invalid JSON'));
    await first;
    assert.equal(dialog.textarea().props.value, content);
    assert.equal(dialog.textarea().props.readOnly, false);

    const corrected = content + '\n';
    dialog.textarea().props.onChange({ target: { value: corrected } });
    assert.equal(dialog.textarea().props.value, corrected);
    const retry = dialog.submit();
    assert.equal(dialog.requests[1].content, corrected);
    assert.equal(dialog.textarea().props.value.length, 0);
    dialog.requests[1].result.resolve();
    await retry;
    assert.equal(dialog.textarea(), undefined);
    dialog.h.unmount();
  });
}
