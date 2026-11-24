import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

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
const API_RELAY_ORDER = 4;
const GROUP_ENTRY_ID = 'group:assistants';
const GROUP_PLATFORM_IDS = ['workbuddy_ai', 'claude_manager'];

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

function renderLayoutModal(sidebarEntryIds: string[]) {
  const orderedEntryIds = [
    GROUP_ENTRY_ID,
    ...PLATFORM_IDS.filter((id) => !GROUP_PLATFORM_IDS.includes(id)).map((id) => `platform:${id}`),
  ];

  // Records calls in invocation order so the assertions can compare exact arrays.
  const events: any[] = [];
  const storeState: any = {
    orderedEntryIds,
    hiddenEntryIds: [],
    sidebarEntryIds,
    trayPlatformIds: [],
    platformGroups: [{
      id: 'assistants',
      name: 'Assistants',
      platformIds: GROUP_PLATFORM_IDS,
      defaultPlatformId: GROUP_PLATFORM_IDS[0],
      iconKind: 'platform',
      iconPlatformId: GROUP_PLATFORM_IDS[0],
    }],
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

  const t = (key: string, options?: { count?: number; max?: number }) => {
    if (options?.count === undefined) return key;
    return options.max === undefined
      ? `${key}(${options.count})`
      : `${key}(${options.count}/${options.max})`;
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
      parseGroupEntryId: (id: string) => id === GROUP_ENTRY_ID ? 'assistants' : null,
      getGroupChildConfig: () => undefined,
      resolveGroupChildName: (_group: unknown, _platformId: string, fallback: string) => fallback,
      resolveGroupChildIcon: () => ({ iconKind: 'platform', iconPlatformId: 'codex' }),
      resolveEntryDefaultPlatformId: (entryId: string) =>
        entryId === GROUP_ENTRY_ID ? GROUP_PLATFORM_IDS[0] : parsePlatformEntryId(entryId),
      resolveEntryPlatformIds: (entryId: string) => {
        if (entryId === GROUP_ENTRY_ID) return GROUP_PLATFORM_IDS;
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

  const tree = h.render(() => h.exports.PlatformLayoutModal({ open: true, onClose: () => {} }));
  return { h, tree, events, orderedEntryIds, storeState };
}

function sidebarCheckboxes(tree: any) {
  return findCheckboxes(tree).filter((item) => item.labelText === 'platformLayout.sidebarToggle');
}

test('sidebar bulk selection enables every entry past the former 3 / 6 caps', () => {
  const { h, tree, events, orderedEntryIds, storeState } = renderLayoutModal([GROUP_ENTRY_ID]);
  const [bulkCheckbox] = sidebarCheckboxes(tree);
  assert.equal(bulkCheckbox.inBulkHeader, true);
  assert.equal(bulkCheckbox.element.props.checked, false);
  assert.match(elementText(tree), /sidebarSelectedUnlimited\(1\)/);
  assert.doesNotMatch(elementText(tree), /\d+\/\d+/, 'summary must not render count/max any more');

  bulkCheckbox.element.props.onChange();
  const expectedEvents = [
    ...orderedEntryIds.slice(0, API_RELAY_ORDER).map((id) => ['sidebar', id, true]),
    ['apiRelay', true],
    ...orderedEntryIds.slice(API_RELAY_ORDER).map((id) => ['sidebar', id, true]),
  ].filter((event) => event[1] !== GROUP_ENTRY_ID);
  assert.deepEqual(events, expectedEvents);
  assert.ok(events.length > 6, 'the former caps (original 3 / classic 6) must not truncate the sweep');
  assert.ok(storeState.sidebarEntryIds.includes(GROUP_ENTRY_ID));

  const fullTree = h.flush();
  const expandGroup = (element: any) => {
    if (element.type === 'button' && element.props.className === 'platform-layout-expand-trigger' && !element.props.disabled) {
      element.props.onClick({ stopPropagation() {} });
    }
  };
  walkTree(fullTree, expandGroup);
  const expandedTree = h.flush();
  const [checkedBulk, group, ...platforms] = sidebarCheckboxes(expandedTree);
  assert.equal(checkedBulk.element.props.checked, true);
  assert.match(elementText(expandedTree), /sidebarSelectedUnlimited\(\d+\)/);
  assert.doesNotMatch(elementText(expandedTree), /\d+\/\d+/);
  assert.equal(group.element.props.checked, true);
  assert.equal(platforms.length, orderedEntryIds.length + GROUP_PLATFORM_IDS.length);
  // No entry is capped by the selection count any more, so nothing gets disabled.
  assert.ok(platforms.every(({ element }) => !element.props.disabled));
  assert.equal(group.element.props.disabled, undefined);

  const [fullBulk] = sidebarCheckboxes(h.flush());
  fullBulk.element.props.onChange();
  assert.deepEqual(storeState.sidebarEntryIds, []);
  assert.equal(storeState.apiRelaySidebarVisible, false);
  h.unmount();
});

test('saved selections beyond the former caps stay removable', () => {
  const previouslySelected = PLATFORM_IDS.slice(0, 7).map((id) => `platform:${id}`);
  const { h, tree, storeState } = renderLayoutModal(previouslySelected);
  assert.match(elementText(tree), /sidebarSelectedUnlimited\(7\)/);
  const [, ...rows] = sidebarCheckboxes(tree);
  const selectedRow = rows.find(({ element }) => element.props.checked);
  assert.ok(selectedRow);
  assert.ok(rows.every(({ element }) => !element.props.disabled));
  selectedRow.element.props.onChange({ target: { checked: false } });
  assert.equal(storeState.sidebarEntryIds.length, 6);
  h.unmount();
});
