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

/** Codex 专用：合并所有历史键解析布局模式 */
export function readCodexOverviewLayoutMode(): AccountsViewMode {
  return readAccountsViewMode('codex', {
    allowCompact: true,
    fallback: 'grid',
    extraLegacyKeys: [
      CODEX_OVERVIEW_LAYOUT_MODE_KEY,
      'agtools.codex.accounts_view_mode',
    ],
  });
}

export function writeCodexOverviewLayoutMode(mode: AccountsViewMode): void {
  const normalized =
    normalizeAccountsViewMode(mode, { allowCompact: true }) ?? 'grid';
  writeAccountsViewMode('codex', normalized, {
    syncFilterField: true,
    extraKeys: [CODEX_OVERVIEW_LAYOUT_MODE_KEY, 'agtools.codex.accounts_view_mode'],
  });
}
