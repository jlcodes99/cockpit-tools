import type { CodexTab } from '../components/CodexOverviewTabsHeader';

export type CodexTopTabPlacement = 'top' | 'more';

export interface CodexTopLayoutPreference {
  order: CodexTab[];
  placement: Record<CodexTab, CodexTopTabPlacement>;
}

export const CODEX_TOP_LAYOUT_STORAGE_KEY = 'agtools.codex.top_layout.v1';
export const CODEX_TOP_LAYOUT_UPDATED_EVENT = 'codex:top-layout-updated';
export const CODEX_TOP_TAB_LIMIT = 5;

export const CODEX_TOP_LAYOUT_TABS: CodexTab[] = [
  'overview',
  'providers',
  'wakeup',
  'instances',
  'sessions',
  'proxy',
  'top-layout',
];

export function createDefaultCodexTopLayout(availableTabs: CodexTab[] = CODEX_TOP_LAYOUT_TABS): CodexTopLayoutPreference {
  return normalizeCodexTopLayout({ order: availableTabs }, availableTabs);
}

export function normalizeCodexTopLayout(value: unknown, availableTabs: CodexTab[] = CODEX_TOP_LAYOUT_TABS): CodexTopLayoutPreference {
  const candidate = (value && typeof value === 'object' ? value : {}) as {
    order?: unknown;
  };
  const validTabs = new Set<CodexTab>(availableTabs);
  const order = Array.isArray(candidate.order)
    ? candidate.order.filter(
        (item, index, items): item is CodexTab =>
          typeof item === 'string' &&
          validTabs.has(item as CodexTab) &&
          items.indexOf(item) === index,
      )
    : [];
  for (const tab of availableTabs) {
    if (!order.includes(tab)) {
      order.push(tab);
    }
  }

  // Placement is derived, never an independent preference (including legacy data).
  const placement = Object.fromEntries(order.map((tab, index) => [
    tab, index < CODEX_TOP_TAB_LIMIT ? 'top' : 'more',
  ])) as CodexTopLayoutPreference['placement'];

  return { order, placement };
}

export function moveCodexTopLayoutTab(
  layout: CodexTopLayoutPreference,
  fromIndex: number,
  toIndex: number,
  availableTabs: CodexTab[] = CODEX_TOP_LAYOUT_TABS,
): CodexTopLayoutPreference {
  const normalized = normalizeCodexTopLayout(layout, availableTabs);
  if (!Number.isInteger(fromIndex) || !Number.isInteger(toIndex)
    || fromIndex < 0 || toIndex < 0
    || fromIndex >= normalized.order.length || toIndex >= normalized.order.length) return normalized;
  const [tab] = normalized.order.splice(fromIndex, 1);
  normalized.order.splice(toIndex, 0, tab);
  return normalizeCodexTopLayout(normalized, availableTabs);
}

/** Strict read for callers that must surface unavailable or corrupt storage. */
export function loadCodexTopLayoutPreference(): CodexTopLayoutPreference {
  const raw = localStorage.getItem(CODEX_TOP_LAYOUT_STORAGE_KEY);
  if (!raw) return createDefaultCodexTopLayout();
  const parsed: unknown = JSON.parse(raw);
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)
    || !Array.isArray((parsed as { order?: unknown }).order)
    || !(parsed as { order: unknown[] }).order.every(tab => typeof tab === 'string')) {
    throw new Error('invalid_codex_top_layout');
  }
  return normalizeCodexTopLayout(parsed);
}

export function readCodexTopLayoutPreference(): CodexTopLayoutPreference {
  try {
    return loadCodexTopLayoutPreference();
  } catch {
    return createDefaultCodexTopLayout();
  }
}

/** Failure propagates; only persisted changes are broadcast to other mounted views. */
export function writeCodexTopLayoutPreference(layout: CodexTopLayoutPreference): void {
  const normalized = normalizeCodexTopLayout(layout);
  localStorage.setItem(CODEX_TOP_LAYOUT_STORAGE_KEY, JSON.stringify({ order: normalized.order }));
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent(CODEX_TOP_LAYOUT_UPDATED_EVENT, { detail: normalized }));
  }
}
