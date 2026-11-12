import { invoke } from '@tauri-apps/api/core';
import { ALL_PLATFORM_IDS, type PlatformId } from '../types/platform';
import {
  CODEX_API_STATS_RANGE_STORAGE_KEY,
  CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY,
  CODEX_SESSION_USAGE_RANGE_STORAGE_KEY,
  isValidCodexStatsRangePreference,
} from './codexStatsRangePreference';

interface Snapshot {
  values: Record<string, string>;
  revisions?: Record<string, number>;
}

const bool = (value: string) => value === '0' || value === '1';
export const FLOATING_CARD_PLATFORM_STORAGE_KEY = 'agtools.floating_card.platform';
const topLayout = (value: string) => {
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed?.order) && parsed.order.every((tab: unknown) => typeof tab === 'string');
  } catch {
    return false;
  }
};

const jsonObject = (value: string) => {
  try {
    const parsed = JSON.parse(value);
    return parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed);
  } catch {
    return false;
  }
};

const jsonArrayOrNull = (value: string) => {
  try {
    return value === 'null' || Array.isArray(JSON.parse(value));
  } catch {
    return false;
  }
};

type SettingSpec = {
  valid: (value: string) => boolean;
  event?: string;
  detail?: (value: string | null) => unknown;
  portable?: false;
};

const settings = {
  [FLOATING_CARD_PLATFORM_STORAGE_KEY]: {
    valid: (value: string) => ALL_PLATFORM_IDS.includes(value as PlatformId),
    portable: false,
  },
  'agtools.breakout.history': { valid: value => value !== 'null' && jsonArrayOrNull(value), portable: false },
  'agtools.breakout.drop_icon_assign.v1': { valid: jsonObject, portable: false },
  'agtools.privacy_mode_enabled': {
    valid: bool, event: 'agtools:privacy-mode-changed', detail: (value: string | null) => value === '1',
  },
  'agtools.codex.top_layout.v1': {
    valid: topLayout, event: 'codex:top-layout-updated',
  },
  'agtools.codex_show_code_review_quota': {
    valid: bool, event: 'agtools:codex-code-review-quota-visibility-changed', detail: (value: string | null) => value === '1',
  },
  'agtools.codex_show_additional_quota': {
    valid: bool, event: 'agtools:codex-additional-quota-visibility-changed', detail: (value: string | null) => value !== '0',
  },
  'agtools.codex_plan_badge_style': {
    valid: (value: string) => ['default', 'outline', 'soft', 'mono'].includes(value),
    event: 'agtools:codex-plan-badge-style-changed', detail: (value: string | null) => value ?? 'default',
  },
  'agtools.claude.quota_display_remaining.v1': {
    valid: bool, event: 'claude-quota-display-remaining-changed',
  },
  'agtools.claude.accounts_view_mode': {
    valid: (value: string) => ['compact', 'list', 'grid'].includes(value),
  },
  'agtools.codex.import.sync_api_service.v1': {
    valid: (value: string) => value === 'true' || value === 'false',
  },
  'agtools.codex.accounts.overview_layout_mode': {
    valid: (value: string) => ['compact', 'list', 'grid'].includes(value),
  },
  'agtools.codex.local_access_entry_expanded.v1': { valid: bool },
  'agtools.codex.local_access_address_kind.v1': {
    valid: (value: string) => value === 'local' || value === 'lan',
    portable: false,
  },
  'agtools.codex.local_access.address_kind.v1': {
    valid: (value: string) => value === 'local' || value === 'lan',
    portable: false,
  },
  'agtools.codex.api_service.request_log_page_size.v1': {
    valid: (value: string) => ['20', '50', '100'].includes(value),
  },
  'codex.apiService.logColumns.v1': {
    valid: (value: string) => value !== 'null' && jsonArrayOrNull(value),
  },
  [CODEX_API_STATS_RANGE_STORAGE_KEY]: { valid: isValidCodexStatsRangePreference },
  [CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY]: { valid: isValidCodexStatsRangePreference },
  [CODEX_SESSION_USAGE_RANGE_STORAGE_KEY]: {
    valid: (value: string) => ['today', '7d', '30d', 'month', 'all'].includes(value),
  },
  'codex.proxyImportProtocol': {
    valid: (value: string) => ['http', 'https', 'socks5', 'socks5h'].includes(value),
  },
  'agtools.codex.wakeup.model_selection': { valid: jsonObject },
  'agtools.codex.modelProviders.custom_sort_active.v1': { valid: bool },
  'agtools.codex.modelProviders.custom_sort_order.v1': {
    valid: (value: string) => value !== 'null' && jsonArrayOrNull(value),
  },
  'agtools.codex.accounts.custom_sort_active.v1': { valid: bool, portable: false },
  'agtools.antigravity.accounts.custom_sort_active.v1': { valid: bool, portable: false },
  'cockpit.codex.cli.lastWorkingDir': {
    valid: (value: string) => value.trim().length > 0,
    portable: false,
  },
  'agtools.codex_proxy_display': {
    valid: (value: string) => ['compact', 'summary', 'detailed'].includes(value),
    event: 'agtools:codex-proxy-display-changed',
  },
  'agtools.claude.cli.last_working_dir': {
    valid: (value: string) => value.trim().length > 0,
    portable: false,
  },
  'agtools.antigravity.runtime_target.v1': {
    valid: (value: string) => value === 'antigravity' || value === 'antigravity_ide',
    event: 'agtools-antigravity-runtime-target-changed',
    detail: (value: string | null) => value ?? 'antigravity_ide',
  },
  'agtools.current_account_refresh_minutes.v1': {
    valid: jsonObject, event: 'agtools:current-account-refresh-preferences-changed',
    portable: false, // Keep exporting the legacy current_account_refresh_minutes field.
  },
  'agtools.account_refresh_overrides.v1': {
    valid: jsonObject, event: 'agtools:current-account-refresh-preferences-changed',
  },
  'agtools.unlock.antigravity_seamless_switch.v1': {
    valid: bool, event: 'agtools:feature-unlock-changed',
    detail: (value: string | null) => ({ feature: 'antigravity.seamless_switch', unlocked: value === '1' }),
  },
  'agtools.ui_config.v1': {
    valid: jsonObject, event: 'agtools:ui-time-display-changed',
  },
  'agtools.side_nav.layout.v1': {
    valid: (value: string) => value === 'classic' || value === 'original',
    event: 'agtools:side-nav-preferences-changed',
  },
  'agtools.side_nav.classic_collapsed.v1': {
    valid: bool, event: 'agtools:side-nav-preferences-changed',
  },
  'agtools.side_nav.hide_classic_switch_prompt.v1': {
    valid: bool, event: 'agtools:side-nav-preferences-changed', portable: false,
  },
  'agtools.side_nav.classic_first_sync_done.v1': {
    valid: bool, event: 'agtools:side-nav-preferences-changed', portable: false,
  },
  'agtools.platform_layout.custom_icons.v1': {
    valid: jsonArrayOrNull, event: 'agtools:platform-layout-icons-changed', portable: false,
  },
  compactGroupOrder: {
    valid: jsonArrayOrNull, event: 'agtools:compact-groups-changed', portable: false,
  },
  compactGroupColors: {
    valid: (value: string) => value === 'null' || jsonObject(value),
    event: 'agtools:compact-groups-changed', portable: false,
  },
  compactHiddenGroups: {
    valid: jsonArrayOrNull, event: 'agtools:compact-groups-changed', portable: false,
  },
} satisfies Record<string, SettingSpec>;

type FixedSettingKey = keyof typeof settings;
export type PageSizeSettingKey = `agtools.${string}.accounts_page_size` | 'codex-image-account-picker-page-size';
export type OverviewFilterFlagKey = `agtools.${string}.accounts_overview_filters.persist_enabled`;
export type OverviewFilterValueKey = `agtools.${string}.accounts_overview_filters.${string}`;
export type ActiveGroupSettingKey = `agtools.group_tab_active.${string}`;
export type FlowNoticeSettingKey = `agtools.${string}.flow_notice_collapsed`;
type InstanceSortFieldKey = `agtools.${string}.instances.sort_field`;
type InstanceSortDirectionKey = `agtools.${string}.instances.sort_direction`;
export type DurableUiSettingKey =
  FixedSettingKey | PageSizeSettingKey | OverviewFilterValueKey | ActiveGroupSettingKey | FlowNoticeSettingKey
  | InstanceSortFieldKey | InstanceSortDirectionKey;
type SettingKey = DurableUiSettingKey;
const fixedKeys = Object.keys(settings) as FixedSettingKey[];
export const PAGE_SIZE_PREFERENCE_CHANGED_EVENT = 'agtools:page-size-preference-changed';
const PAGE_SIZE_SPEC: SettingSpec = {
  valid: value => /^[1-9][0-9]*$/.test(value) && Number.isSafeInteger(Number(value)),
  event: PAGE_SIZE_PREFERENCE_CHANGED_EVENT,
};
const FILTER_FLAG_PATTERN = /^agtools\.([a-z0-9_]+)\.accounts_overview_filters\.persist_enabled$/;
const FILTER_FLAG_SPEC: SettingSpec = {
  valid: bool,
  event: 'agtools:accounts-overview-filter-persistence-changed',
};
const ACTIVE_GROUP_SPEC: SettingSpec = {
  valid: value => value.trim().length > 0,
  portable: false,
};
const FLOW_NOTICE_PATTERN = /^agtools\.[a-z0-9_]+\.flow_notice_collapsed$/;
const FLOW_NOTICE_SPEC: SettingSpec = { valid: bool };
const INSTANCE_SORT_FIELD_SPEC: SettingSpec = { valid: value => value === 'createdAt' || value === 'lastLaunchedAt' };
const INSTANCE_SORT_DIRECTION_SPEC: SettingSpec = { valid: value => value === 'asc' || value === 'desc' };
const FILTER_FIELD_PATTERN = /^agtools\.[a-z0-9_]+\.accounts_overview_filters\.(view_mode|sort_by|sort_direction|filter_type|filter_types|tag_filter|tags|group_by_tag|group_filter|search_query)$/;
export const OVERVIEW_FILTER_VALUES_CHANGED_EVENT = 'agtools:accounts-overview-filter-values-changed';
const FILTER_FIELD_SPEC: SettingSpec = {
  valid: value => {
    try {
      JSON.parse(value);
      return true;
    } catch {
      return false;
    }
  },
  event: OVERVIEW_FILTER_VALUES_CHANGED_EVENT,
  portable: false,
};
const SIDE_NAV_LEGACY_FIELDS: Record<string, string> = {
  'agtools.side_nav.layout.v1': 'mode',
  'agtools.side_nav.classic_collapsed.v1': 'classicCollapsed',
  'agtools.side_nav.hide_classic_switch_prompt.v1': 'hideClassicSwitchPrompt',
  'agtools.side_nav.classic_first_sync_done.v1': 'classicFirstSyncDone',
};
const LEGACY_ANTIGRAVITY_SORT_FIELDS = {
  'agtools.antigravity.accounts_overview_filters.sort_by': 'accountsSortBy',
  'agtools.antigravity.accounts_overview_filters.sort_direction': 'accountsSortDirection',
} as const;

function settingFor(key: string): SettingSpec | undefined {
  if (Object.prototype.hasOwnProperty.call(settings, key)) return settings[key as FixedSettingKey];
  if (/^agtools\.[a-z0-9_]+\.accounts_page_size$/.test(key) || key === 'codex-image-account-picker-page-size') {
    return PAGE_SIZE_SPEC;
  }
  if (FILTER_FLAG_PATTERN.test(key)) return FILTER_FLAG_SPEC;
  if (FILTER_FIELD_PATTERN.test(key)) return FILTER_FIELD_SPEC;
  if (/^agtools\.group_tab_active\.[a-z0-9_-]+$/.test(key)) return ACTIVE_GROUP_SPEC;
  if (FLOW_NOTICE_PATTERN.test(key)) return FLOW_NOTICE_SPEC;
  if (/^agtools\.[a-z0-9_-]+\.instances\.sort_field$/.test(key)) return INSTANCE_SORT_FIELD_SPEC;
  if (/^agtools\.[a-z0-9_-]+\.instances\.sort_direction$/.test(key)) return INSTANCE_SORT_DIRECTION_SPEC;
  return undefined;
}

function knownKeys(...sources: string[][]): SettingKey[] {
  return [...new Set([...fixedKeys, ...Object.keys(LEGACY_ANTIGRAVITY_SORT_FIELDS), 'agtools.group_tab_active.codex', ...sources.flat()])]
    .filter(key => settingFor(key)) as SettingKey[];
}

const IPC_TIMEOUT_MS = 3000;

function withDeadline<T>(operation: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout>;
  return Promise.race([
    operation,
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('ui_preference_timeout')), IPC_TIMEOUT_MS);
    }),
  ]).finally(() => clearTimeout(timer));
}

let snapshot: Snapshot | null = null;
let hydration: Promise<void> | null = null;
let hydrated = false;
let invalidLegacyTopLayout: string | null = null;
const queuedByKey = new Map<SettingKey, Promise<number>>();
let writes = Promise.resolve();
const pending = new Map<SettingKey, { value: string }>();
const subscribers = new Map<SettingKey, Set<() => void>>();

export function subscribeDurableUiSetting(key: SettingKey, listener: () => void): () => void {
  const listeners = subscribers.get(key) ?? new Set();
  subscribers.set(key, listeners);
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (!listeners.size) subscribers.delete(key);
  };
}

function legacyStorage(): Storage | undefined {
  return globalThis.window?.localStorage ?? globalThis.localStorage;
}

function legacyKeys(): string[] {
  try {
    const storage = legacyStorage();
    if (!storage) return [];
    return Array.from({ length: storage.length }, (_, index) => storage.key(index))
      .filter((key): key is string => key !== null);
  } catch {
    return [];
  }
}

function readLegacy(key: SettingKey): string | null {
  try {
    const storage = legacyStorage();
    if (Object.prototype.hasOwnProperty.call(SIDE_NAV_LEGACY_FIELDS, key)) {
      const old = storage?.getItem('agtools.side_nav.store.v2');
      if (old) {
        try {
          const value = JSON.parse(old)?.state?.[SIDE_NAV_LEGACY_FIELDS[key]];
          if (typeof value === 'boolean') return value ? '1' : '0';
          if (typeof value === 'string' && settingFor(key)?.valid(value)) return value;
        } catch { /* Earlier individual keys can still be read. */ }
      }
    }
    const value = storage?.getItem(key) ?? null;
    if (value === null && Object.prototype.hasOwnProperty.call(LEGACY_ANTIGRAVITY_SORT_FIELDS, key)) {
      const raw = storage?.getItem(LEGACY_ANTIGRAVITY_SORT_FIELDS[key as keyof typeof LEGACY_ANTIGRAVITY_SORT_FIELDS]);
      return raw?.trim() ? JSON.stringify(raw.trim()) : null;
    }
    if (FLOW_NOTICE_PATTERN.test(key)) {
      if (value === 'true') return '1';
      if (value === 'false') return '0';
    }
    if (key === 'agtools.codex.accounts.overview_layout_mode' && (value === null || !settingFor(key)?.valid(value))) {
      const previous = storage?.getItem('agtools.codex.accounts_view_mode');
      return previous === 'list' || previous === 'grid' ? previous : null;
    }
    if (key === 'agtools.group_tab_active.codex' && value === null) {
      const previous = storage?.getItem('agtools.codex.accounts_overview_filters.active_group_id');
      if (previous) {
        const group: unknown = JSON.parse(previous);
        if (typeof group === 'string') return group;
      }
    }
    return value;
  } catch {
    return null;
  }
}

export function readDurableUiSetting(key: SettingKey): string | null {
  const preview = pending.get(key);
  if (preview) return preview.value;
  if (snapshot?.values[key] !== undefined) return snapshot.values[key];
  if (hydrated) return key === 'agtools.codex.top_layout.v1' ? invalidLegacyTopLayout : null;
  if (key === 'agtools.privacy_mode_enabled') return '1';
  const legacy = readLegacy(key);
  return legacy !== null && (key === 'agtools.codex.top_layout.v1' || settingFor(key)?.valid(legacy))
    ? legacy : null;
}

export function areDurableUiSettingsHydrated(): boolean {
  return hydrated;
}

export function readCommittedDurableUiSetting(key: OverviewFilterFlagKey): string | null {
  return snapshot?.values[key] ?? null;
}

function notify(key: SettingKey): void {
  subscribers.get(key)?.forEach(listener => listener());
  if (typeof window === 'undefined') return;
  const setting = settingFor(key);
  if (!setting?.event) return;
  const value = readDurableUiSetting(key);
  const detail = setting === FILTER_FLAG_SPEC
    ? { scope: FILTER_FLAG_PATTERN.exec(key)?.[1], enabled: value !== '0' }
    : setting === PAGE_SIZE_SPEC ? { key } : setting.detail?.(value);
  window.dispatchEvent(new CustomEvent(setting.event, { detail }));
}

export function applyDurableUiSnapshot(incoming: Snapshot, fromHydration = false): void {
  const keys = knownKeys(hydrated ? [] : legacyKeys(), Object.keys(snapshot?.values ?? {}), Object.keys(incoming.values));
  const before = new Map(keys.map(key => [key, readDurableUiSetting(key)]));
  const next: Snapshot = { values: { ...snapshot?.values }, revisions: { ...snapshot?.revisions } };
  let filterFlagCommitted = false;
  for (const key of keys) {
    const value = incoming.values[key];
    const revision = incoming.revisions?.[key] ?? 0;
    if (value !== undefined && revision >= (next.revisions?.[key] ?? 0)) {
      if (FILTER_FLAG_PATTERN.test(key) && next.values[key] !== value) filterFlagCommitted = true;
      next.values[key] = value;
      next.revisions![key] = revision;
    }
  }
  snapshot = next;
  if (fromHydration) hydrated = true;
  for (const key of keys) {
    if (before.get(key) !== readDurableUiSetting(key)) notify(key);
  }
  if ((fromHydration || filterFlagCommitted) && typeof window !== 'undefined') {
    window.dispatchEvent(new Event(OVERVIEW_FILTER_VALUES_CHANGED_EVENT));
  }
}

export function hydrateDurableUiSettings(): Promise<void> {
  if (hydration) return withDeadline(hydration);
  if (hydrated) return Promise.resolve();
  const legacy: Record<string, string> = {};
  for (const key of knownKeys(legacyKeys())) {
    const value = readLegacy(key);
    if (value !== null && settingFor(key)?.valid(value)) legacy[key] = value;
    else if (key === 'agtools.codex.top_layout.v1') invalidLegacyTopLayout = value;
  }
  hydration = invoke<Snapshot>('migrate_ui_preference_values', { legacy })
    .then(loaded => applyDurableUiSnapshot(loaded, true))
    .finally(() => { hydration = null; });
  return withDeadline(hydration);
}

export function saveDurableUiSetting(key: SettingKey, value: string, optimistic = false): Promise<void> {
  if (!settingFor(key)?.valid(value)) return Promise.reject(new Error('invalid_ui_preference'));
  const preview = { value };
  const deadline = Date.now() + IPC_TIMEOUT_MS;
  // Capture the revision visible when the user edits. Earlier writes in this
  // window can advance it, while an unrelated window cannot silently rebase it.
  const visibleRevision = snapshot?.revisions?.[key];
  const earlier = queuedByKey.get(key);
  if (optimistic) {
    pending.set(key, preview);
    notify(key);
  }
  const operation = writes.then(async () => {
    await hydrateDurableUiSettings();
    const expectedRevision = earlier
      ? await earlier
      : visibleRevision ?? snapshot?.revisions?.[key] ?? 0;
    if (Date.now() >= deadline) throw new Error('ui_preference_timeout');
    try {
      const saved = await invoke<Snapshot>('update_ui_preference', { key, value, expectedRevision });
      applyDurableUiSnapshot(saved);
      return saved.revisions?.[key] ?? expectedRevision;
    } catch (error) {
      if (String(error).includes('ui_preference_revision_conflict')) {
        const current = await invoke<Snapshot>('load_ui_preferences');
        applyDurableUiSnapshot(current);
      }
      throw error;
    }
  });
  queuedByKey.set(key, operation);
  const settled = operation.then(() => {}).finally(() => {
    if (queuedByKey.get(key) === operation) queuedByKey.delete(key);
    if (pending.get(key) === preview) {
      const before = readDurableUiSetting(key);
      pending.delete(key);
      if (readDurableUiSetting(key) !== before) notify(key);
    }
  });
  writes = settled.catch(() => {});
  // The caller can time out, while an already-issued native write continues
  // to own the queue until it settles and publishes its real result.
  return withDeadline(settled);
}

export function previewDurableUiSetting(key: SettingKey, value: string): void {
  void saveDurableUiSetting(key, value, true).catch(error => console.warn('[UI preferences]', error));
}

function isPortable(key: SettingKey): boolean {
  return key !== 'agtools.codex.top_layout.v1' && settingFor(key)?.portable !== false;
}

export async function exportDurableUiSettings(): Promise<Record<string, string>> {
  await hydrateDurableUiSettings();
  return Object.fromEntries(knownKeys(Object.keys(snapshot?.values ?? {})).flatMap(key => {
    const value = snapshot?.values[key];
    return isPortable(key) && value !== undefined ? [[key, value]] : [];
  }));
}

export async function importDurableUiSettings(values: Record<string, unknown>): Promise<void> {
  if (!values || typeof values !== 'object' || Array.isArray(values)) throw new Error('invalid_ui_preference');
  const keys = knownKeys(Object.keys(values));
  for (const key of keys) {
    const value = values[key];
    if (!isPortable(key) || value === undefined) continue;
    if (typeof value !== 'string' || !settingFor(key)?.valid(value)) throw new Error('invalid_ui_preference');
  }
  for (const key of keys) {
    const value = values[key];
    if (isPortable(key) && typeof value === 'string') await saveDurableUiSetting(key, value);
  }
}
