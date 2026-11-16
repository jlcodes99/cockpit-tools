import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

// Bulk "select sidebar" used to be capped at ORIGINAL_SIDEBAR_ENTRY_LIMIT (3) /
// CLASSIC_SIDEBAR_ENTRY_LIMIT (6). Those constants lived in
// src/stores/useSideNavLayoutStore.ts and were removed by "feat(sidebar): remove
// selection and display limits in both layouts", so after mocking the store the
// old cap is no longer readable at runtime. The dataset below (20 platform
// entries) is deliberately far above the former cap: the old implementation
// would have stopped at 6 setters instead of touching every entry.
const PLATFORM_IDS = [
  'codex',
  'antigravity',
  'zed',
  'github-copilot',
  'windsurf',
  'kiro',
  'cursor',
  'grok',
  'codebuddy',
  'codebuddy_cn',
  'qoder',
  'zcode',
  'trae',
  'trae_solo',
  'trae_cn',
  'trae_solo_cn',
  'workbuddy',
  'workbuddy_ai',
  'minimaxcode',
  'claude_manager',
];
const API_RELAY_ENTRY_ID = 'feature:api-relay';
const API_RELAY_ORDER = 5;

function isElement(node: any) {
  return !!node && typeof node === 'object' && 'props' in node && typeof node.type !== 'undefined';
}

/** Walks a React element tree, expanding the mocked function components inline. */
function walkTree(node: any, visit: (element: any, ancestors: any[]) => void, ancestors: any[] = []) {
  if (node == null || node === false || node === true) return;
  if (typeof node === 'string' || typeof node === 'number') return;
  if (Array.isArray(node)) {
    for (const child of node) walkTree(child, visit, ancestors);
    return;
  }
  if (!isElement(node)) return;
  visit(node, ancestors);
  if (typeof node.type === 'function') {
    walkTree(node.type(node.props), visit, ancestors);
    return;
  }
  walkTree(node.props.children, visit, [...ancestors, node]);
}

function elementText(node: any): string {
  let text = '';
  const collect = (current: any) => {
    if (current == null || current === false || current === true) return;
    if (typeof current === 'string' || typeof current === 'number') {
      text += String(current);
      return;
    }
    if (Array.isArray(current)) {
      current.forEach(collect);
      return;
    }
    if (!isElement(current)) return;
    if (typeof current.type === 'function') {
      collect(current.type(current.props));
      return;
    }
    collect(current.props.children);
  };
  collect(node);
  return text;
}

function findCheckboxes(tree: any) {
  const checkboxes: { element: any; labelText: string; inBulkHeader: boolean }[] = [];
  walkTree(tree, (element, ancestors) => {
    if (element.type !== 'input' || element.props.type !== 'checkbox') return;
    const label = ancestors[ancestors.length - 1];
    const inBulkHeader = ancestors.some(
      (ancestor) =>
        typeof ancestor.props?.className === 'string'
        && ancestor.props.className.includes('platform-layout-bulk-header'),
    );
    checkboxes.push({
      element,
      labelText: label ? elementText(label.props.children) : '',
      inBulkHeader,
    });
  });
  return checkboxes;
}

test('bulk sidebar toggle enables every entry, including api-relay, without an entry cap', () => {
  const orderedEntryIds = PLATFORM_IDS.map((platformId) => `platform:${platformId}`);
  assert.equal(orderedEntryIds.length, 20);

  // Records calls in invocation order so the assertions can compare exact arrays.
  const events: any[] = [];
  const storeState: any = {
    orderedEntryIds,
    hiddenEntryIds: [],
    sidebarEntryIds: [],
    trayPlatformIds: [],
    platformGroups: [],
    apiRelaySidebarVisible: false,
    apiRelayDashboardVisible: true,
    apiRelayEntryOrder: API_RELAY_ORDER,
    disabledPlatformIds: [],
    reorderGroupPlatforms() {},
    setLayoutEntryOrder() {},
    setHiddenEntry() {},
    setDisabledPlatform() {},
    setSidebarEntry(entryId: string, enabled: boolean) {
      events.push(['sidebar', entryId, enabled]);
      storeState.sidebarEntryIds = enabled
        ? [...storeState.sidebarEntryIds, entryId]
        : storeState.sidebarEntryIds.filter((id: string) => id !== entryId);
    },
    setTrayPlatform() {},
    setApiRelaySidebarVisible(enabled: boolean) {
      events.push(['apiRelay', enabled]);
      storeState.apiRelaySidebarVisible = enabled;
    },
    setApiRelayDashboardVisible() {},
    upsertPlatformGroup() {},
    removePlatformGroup() {},
    resetPlatformLayout() {},
  };

  const tCalls: [string, any][] = [];
  const t = (key: string, optionsOrFallback?: any) => {
    tCalls.push([key, optionsOrFallback]);
    const options = optionsOrFallback && typeof optionsOrFallback === 'object'
      ? optionsOrFallback
      : undefined;
    return options && 'count' in options ? `${key}(${options.count})` : key;
  };

  const parsePlatformEntryId = (entryId: string) =>
    entryId.startsWith('platform:') ? entryId.slice('platform:'.length) : null;

  const h = loadHookModule(new URL('./PlatformLayoutModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '@dnd-kit/sortable': { arrayMove: (items: unknown[]) => items },
    './SortableLayoutList': {
      SortableLayoutList: ({ children }: any) => children,
      SortableLayoutItem: ({ children }: any) => children({}),
    },
    './SingleSelectDropdown': { SingleSelectDropdown: () => null },
    'lucide-react': {
      Check: () => null,
      ChevronDown: () => null,
      ChevronRight: () => null,
      ChevronsDownUp: () => null,
      ChevronsUpDown: () => null,
      GripVertical: () => null,
      Pencil: () => null,
      Plus: () => null,
      Trash2: () => null,
      Upload: () => null,
      X: () => null,
    },
    '../assets/icons/apikey-fun.png': 'api-relay.png',
    '../types/platform': {
      isMenuVisiblePlatform: () => true,
      MENU_VISIBLE_PLATFORM_IDS: PLATFORM_IDS,
    },
    '../stores/useSponsorStore': {
      useSponsorStore: (selector: (state: any) => unknown) =>
        selector({ state: { sponsorModule: true } }),
    },
    '../stores/usePlatformLayoutStore': {
      API_RELAY_LAYOUT_ENTRY_ID: API_RELAY_ENTRY_ID,
      makePlatformEntryId: (platformId: string) => `platform:${platformId}`,
      parsePlatformEntryId,
      parseGroupEntryId: () => null,
      getGroupChildConfig: () => undefined,
      resolveGroupChildName: (_group: unknown, _platformId: string, fallback: string) => fallback,
      resolveGroupChildIcon: () => ({ iconKind: 'platform', iconPlatformId: 'codex' }),
      resolveEntryDefaultPlatformId: (entryId: string) => parsePlatformEntryId(entryId),
      resolveEntryPlatformIds: (entryId: string) => {
        const platformId = parsePlatformEntryId(entryId);
        return platformId ? [platformId] : [];
      },
      usePlatformLayoutStore: () => storeState,
    },
    '../utils/platformMeta': {
      getPlatformLabel: (platformId: string) => platformId,
      renderPlatformIcon: () => null,
    },
    '../hooks/useEscClose': { useEscClose: () => {} },
    '../utils/reducedMotion': { scrollElementIntoView: () => {} },
    '../utils/uiPreferences': {
      getPlatformLayoutPersistenceError: () => null,
      retryPlatformLayoutPersistence: async () => {},
    },
    '../utils/platformLayoutIconStorage': {
      PLATFORM_LAYOUT_ICONS_CHANGED_EVENT: 'platform-layout-icons-changed',
      readPlatformLayoutCustomIcons: () => null,
      persistPlatformLayoutCustomIcons: async () => true,
    },
    '../utils/durableUiSettings': { hydrateDurableUiSettings: async () => {} },
  }, {
    window: { addEventListener() {}, removeEventListener() {} },
  });

  let tree = h.render(() => h.exports.PlatformLayoutModal({ open: true, onClose: () => {} }));

  // Nothing selected yet: the bulk sidebar checkbox is unchecked and the summary
  // renders the unlimited key with the current count (no "/max" suffix).
  let checkboxes = findCheckboxes(tree);
  let bulkSidebar = checkboxes.find((item) => item.inBulkHeader && item.labelText.includes('sidebarToggle'));
  assert.ok(bulkSidebar, 'expected the sidebar bulk checkbox in the bulk header');
  assert.equal(bulkSidebar!.element.props.checked, false);
  assert.equal(tCalls.filter(([key]) => key === 'platformLayout.sidebarSelected').length, 0);
  assert.ok(
    elementText(tree).includes('platformLayout.sidebarSelectedUnlimited(0)'),
    'summary should use platformLayout.sidebarSelectedUnlimited',
  );
  assert.doesNotMatch(elementText(tree), /\d+\/\d+/, 'summary must not render count/max');

  // Toggling the bulk checkbox must hit the setter of every entry, in entry order,
  // with the api-relay entry included at its configured position.
  bulkSidebar!.element.props.onChange();
  const expectedEvents = [
    ...PLATFORM_IDS.slice(0, API_RELAY_ORDER).map((platformId) => ['sidebar', `platform:${platformId}`, true]),
    ['apiRelay', true],
    ...PLATFORM_IDS.slice(API_RELAY_ORDER).map((platformId) => ['sidebar', `platform:${platformId}`, true]),
  ];
  assert.deepEqual(events, expectedEvents);
  assert.equal(events.length, 21, 'all 20 platforms plus api-relay must be enabled');
  // The former caps (3 / 6) would have produced far fewer calls.
  assert.ok(events.length > 6);

  // Re-render against the updated mock store: everything is selected and the
  // summary still uses the unlimited key, now with the full count.
  tree = h.flush();
  checkboxes = findCheckboxes(tree);
  bulkSidebar = checkboxes.find((item) => item.inBulkHeader && item.labelText.includes('sidebarToggle'));
  assert.equal(bulkSidebar!.element.props.checked, true);
  assert.ok(
    elementText(tree).includes('platformLayout.sidebarSelectedUnlimited(21)'),
    'summary should count every selected entry without a max',
  );
  assert.equal(tCalls.filter(([key]) => key === 'platformLayout.sidebarSelected').length, 0);
  assert.doesNotMatch(elementText(tree), /\d+\/\d+/, 'summary must not render count/max');

  h.unmount();
});
