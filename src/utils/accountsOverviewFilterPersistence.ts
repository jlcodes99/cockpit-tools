import {
  previewDurableUiSetting,
  readCommittedDurableUiSetting,
  readDurableUiSetting,
  saveDurableUiSetting,
  type OverviewFilterFlagKey,
  type OverviewFilterValueKey,
} from './durableUiSettings';

export const ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT =
  'agtools:accounts-overview-filter-persistence-changed';

export interface AccountsOverviewFilterPersistenceChangedDetail {
  scope: string;
  enabled: boolean;
}

const QUICK_SETTINGS_SCOPE_MAP: Record<string, string> = {
  antigravity: 'antigravity',
  codex: 'codex',
  claude: 'claude',
  github_copilot: 'githubcopilot',
  windsurf: 'windsurf',
  kiro: 'kiro',
  cursor: 'cursor',
  grok: 'grok',
  codebuddy: 'codebuddy',
  codebuddy_cn: 'codebuddy_cn',
  qoder: 'qoder',
  zcode: 'zcode',
  trae: 'trae',
  trae_solo: 'trae_solo',
  trae_cn: 'trae_cn',
  trae_solo_cn: 'trae_solo_cn',
  workbuddy: 'workbuddy',
  minimaxcode: 'minimaxcode',
  zed: 'zed',
};

function getEnabledStorageKey(scope: string): OverviewFilterFlagKey {
  return `agtools.${scope}.accounts_overview_filters.persist_enabled`;
}

const CLEARED_FIELD = 'null';

function getFieldStorageKey(scope: string, field: string): OverviewFilterValueKey {
  return `agtools.${normalizeAccountsOverviewScope(scope)}.accounts_overview_filters.${field}`;
}

export function readRawAccountsOverviewFilterField(scope: string, field: string): string | null {
  const raw = readDurableUiSetting(getFieldStorageKey(scope, field));
  return raw === CLEARED_FIELD ? null : raw;
}

export function normalizeAccountsOverviewScope(rawScope: string): string {
  const normalized = rawScope.trim().toLowerCase().replace(/[^a-z0-9]+/g, '_');
  return normalized || 'default';
}

export function resolveAccountSortMode(remembered: string, active: string | null, fallback: string): string {
  if (active === '1') return 'custom';
  return active === '0' && remembered === 'custom' ? fallback : remembered;
}

export function resolveAccountsOverviewScopeFromQuickSettingsType(type: string): string {
  return QUICK_SETTINGS_SCOPE_MAP[type] ?? normalizeAccountsOverviewScope(type);
}

export function readAccountsOverviewFilterPersistenceEnabled(rawScope: string): boolean {
  return readDurableUiSetting(getEnabledStorageKey(normalizeAccountsOverviewScope(rawScope))) !== '0';
}

export function isAccountsOverviewFilterPersistenceDisabledOnDisk(scope: string): boolean {
  return readCommittedDurableUiSetting(getEnabledStorageKey(normalizeAccountsOverviewScope(scope))) === '0';
}

export function setAccountsOverviewFilterPersistenceEnabled(rawScope: string, enabled: boolean): void {
  previewDurableUiSetting(getEnabledStorageKey(normalizeAccountsOverviewScope(rawScope)), enabled ? '1' : '0');
}

export function readAccountsOverviewFilterField<T>(
  rawScope: string,
  field: string,
  fallback: T,
): T {
  try {
    const raw = readRawAccountsOverviewFilterField(rawScope, field);
    if (raw == null) {
      return fallback;
    }
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

export function readAccountsOverviewFilterStringArray(
  rawScope: string,
  field: string,
): string[] {
  const value = readAccountsOverviewFilterField<unknown>(rawScope, field, []);
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .filter((item): item is string => typeof item === 'string')
    .map((item) => item.trim())
    .filter(Boolean);
}

export function saveAccountsOverviewFilterField<T>(
  rawScope: string,
  field: string,
  value: T,
): Promise<void> {
  return saveDurableUiSetting(getFieldStorageKey(rawScope, field), JSON.stringify(value), true);
}

export function writeAccountsOverviewFilterField<T>(scope: string, field: string, value: T): void {
  if (readDurableUiSetting(getFieldStorageKey(scope, field)) === JSON.stringify(value)) return;
  void saveAccountsOverviewFilterField(scope, field, value)
    .catch(error => console.warn('[Account overview filters]', error));
}

export function removeAccountsOverviewFilterField(scope: string, field: string): void {
  // Keep an explicit cleared value so retained legacy storage cannot restore it.
  if (readRawAccountsOverviewFilterField(scope, field) !== null) {
    writeAccountsOverviewFilterField(scope, field, null);
  }
}
