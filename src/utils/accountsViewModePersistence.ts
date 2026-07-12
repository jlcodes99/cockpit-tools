/**
 * 账号列表视图模式（列表 / 卡片平铺 / 紧凑）持久化。
 *
 * 与「筛选条件是否持久化」开关解耦：视图布局是 UI 偏好，更新/重载后应始终保留，
 * 不能因 filter persist 关闭或初始化默认 grid 被清掉。
 */

import {
  normalizeAccountsOverviewScope,
  readAccountsOverviewFilterField,
  writeAccountsOverviewFilterField,
} from './accountsOverviewFilterPersistence';

export type AccountsViewMode = 'grid' | 'list' | 'compact';

const FILTER_FIELD_VIEW_MODE = 'view_mode';

/** 专用键：始终读写，不随筛选持久化开关删除 */
export function getAccountsViewModeStorageKey(rawScope: string): string {
  const scope = normalizeAccountsOverviewScope(rawScope);
  return `agtools.${scope}.accounts_view_mode`;
}

/** Codex 历史键 / 概览布局键 */
export const CODEX_OVERVIEW_LAYOUT_MODE_KEY =
  'agtools.codex.accounts.overview_layout_mode';

export function normalizeAccountsViewMode(
  value: unknown,
  options: { allowCompact?: boolean } = {},
): AccountsViewMode | null {
  if (value === 'list' || value === 'grid') return value;
  if (options.allowCompact && value === 'compact') return 'compact';
  if (typeof value === 'string') {
    try {
      const parsed = JSON.parse(value) as unknown;
      if (parsed === 'list' || parsed === 'grid') return parsed;
      if (options.allowCompact && parsed === 'compact') return parsed;
    } catch {
      // plain string already handled
    }
  }
  return null;
}

function safeGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function safeSet(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // ignore
  }
}

/**
 * 读取视图模式：专用键 → 筛选字段 view_mode → 其它 legacy 键。
 * 全部没有时返回 fallback（默认 grid）。
 */
export function readAccountsViewMode(
  rawScope: string,
  options: {
    allowCompact?: boolean;
    fallback?: AccountsViewMode;
    extraLegacyKeys?: string[];
  } = {},
): AccountsViewMode {
  const fallback = options.fallback ?? 'grid';
  const allowCompact = options.allowCompact === true;
  const scope = normalizeAccountsOverviewScope(rawScope);

  const candidates: unknown[] = [
    safeGet(getAccountsViewModeStorageKey(scope)),
    // 筛选持久化字段（JSON 存 "list"/"grid"）
    readAccountsOverviewFilterField<unknown>(scope, FILTER_FIELD_VIEW_MODE, null),
    ...(options.extraLegacyKeys ?? []).map((k) => safeGet(k)),
  ];

  for (const raw of candidates) {
    const mode = normalizeAccountsViewMode(raw, { allowCompact });
    if (mode) return mode;
  }
  return fallback;
}

/**
 * 写入视图模式到专用键；若筛选持久化开启则同步写筛选字段（兼容旧逻辑）。
 * 绝不会在「关闭筛选持久化」时删除专用键。
 */
export function writeAccountsViewMode(
  rawScope: string,
  mode: AccountsViewMode,
  options: {
    syncFilterField?: boolean;
    extraKeys?: string[];
  } = {},
): void {
  const scope = normalizeAccountsOverviewScope(rawScope);
  const normalized =
    normalizeAccountsViewMode(mode, { allowCompact: true }) ?? 'grid';

  safeSet(getAccountsViewModeStorageKey(scope), normalized);

  if (options.syncFilterField) {
    writeAccountsOverviewFilterField(scope, FILTER_FIELD_VIEW_MODE, normalized);
  }

  for (const key of options.extraKeys ?? []) {
    safeSet(key, normalized);
  }
}

/**
 * Codex 专用读取：优先 overview_layout_mode（含 compact），
 * 再读专用 accounts_view_mode / 筛选字段，避免 hook 写入的 list/grid 盖住 compact。
 */
export function readCodexOverviewLayoutMode(): AccountsViewMode {
  const allowCompact = true;
  const candidates: unknown[] = [
    safeGet(CODEX_OVERVIEW_LAYOUT_MODE_KEY),
    safeGet(getAccountsViewModeStorageKey('codex')),
    readAccountsOverviewFilterField<unknown>('codex', FILTER_FIELD_VIEW_MODE, null),
    // 与专用键同名历史路径（兼容旧数据）
    safeGet('agtools.codex.accounts_view_mode'),
  ];
  for (const raw of candidates) {
    const mode = normalizeAccountsViewMode(raw, { allowCompact });
    if (mode) return mode;
  }
  return 'grid';
}

export function writeCodexOverviewLayoutMode(mode: AccountsViewMode): void {
  const normalized =
    normalizeAccountsViewMode(mode, { allowCompact: true }) ?? 'grid';
  writeAccountsViewMode('codex', normalized, {
    syncFilterField: true,
    extraKeys: [CODEX_OVERVIEW_LAYOUT_MODE_KEY],
  });
}

/**
 * 通用平台写入 list/grid 时：若 Codex 当前为 compact，不要用 list/grid 覆盖 overview 键。
 *（Codex 页 hook 与 overviewLayoutMode 双写时的保护）
 */
export function writeAccountsViewModeSafeForCodex(
  rawScope: string,
  mode: AccountsViewMode,
  options: { syncFilterField?: boolean } = {},
): void {
  const scope = normalizeAccountsOverviewScope(rawScope);
  if (scope === 'codex') {
    const currentOverview = normalizeAccountsViewMode(
      safeGet(CODEX_OVERVIEW_LAYOUT_MODE_KEY),
      { allowCompact: true },
    );
    if (currentOverview === 'compact' && mode !== 'compact') {
      // 仅更新 list/grid 备用键，保留 compact 作为当前布局
      safeSet(getAccountsViewModeStorageKey(scope), mode === 'list' ? 'list' : 'grid');
      if (options.syncFilterField) {
        writeAccountsOverviewFilterField(
          scope,
          FILTER_FIELD_VIEW_MODE,
          mode === 'list' ? 'list' : 'grid',
        );
      }
      return;
    }
    writeCodexOverviewLayoutMode(mode);
    return;
  }
  writeAccountsViewMode(rawScope, mode, options);
}
