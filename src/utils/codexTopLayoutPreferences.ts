import type { CodexTab } from '../components/CodexOverviewTabsHeader';

export type CodexTopTabPlacement = 'top' | 'more';

export interface CodexTopLayoutPreference {
  order: CodexTab[];
  placement: Record<CodexTab, CodexTopTabPlacement>;
}

const STORAGE_KEY = 'agtools.codex.top_layout.v1';
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

export function createDefaultCodexTopLayout(): CodexTopLayoutPreference {
  return normalizeCodexTopLayout({ order: CODEX_TOP_LAYOUT_TABS });
}

export function normalizeCodexTopLayout(value: unknown): CodexTopLayoutPreference {
  const candidate = (value && typeof value === 'object' ? value : {}) as {
    order?: unknown;
  };
  const validTabs = new Set<CodexTab>(CODEX_TOP_LAYOUT_TABS);
  const order = Array.isArray(candidate.order)
    ? candidate.order.filter(
        (item, index, items): item is CodexTab =>
          typeof item === 'string' &&
          validTabs.has(item as CodexTab) &&
          items.indexOf(item) === index,
      )
    : [];
  for (const tab of CODEX_TOP_LAYOUT_TABS) {
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
): CodexTopLayoutPreference {
  const normalized = normalizeCodexTopLayout(layout);
  if (!Number.isInteger(fromIndex) || !Number.isInteger(toIndex)
    || fromIndex < 0 || toIndex < 0
    || fromIndex >= normalized.order.length || toIndex >= normalized.order.length) return normalized;
  const [tab] = normalized.order.splice(fromIndex, 1);
  normalized.order.splice(toIndex, 0, tab);
  return normalizeCodexTopLayout(normalized);
}

export function readCodexTopLayoutPreference(): CodexTopLayoutPreference {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? normalizeCodexTopLayout(JSON.parse(raw)) : createDefaultCodexTopLayout();
  } catch {
    return createDefaultCodexTopLayout();
  }
}

export function writeCodexTopLayoutPreference(layout: CodexTopLayoutPreference): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ order: normalizeCodexTopLayout(layout).order }));
  } catch {
    // ignore localStorage write failures
  }
}
