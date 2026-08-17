/**
 * Codex 账号分组服务
 * 数据通过 Tauri 命令持久化到磁盘 (~/.antigravity_cockpit/codex_account_groups.json)
 * 内存中维护一份缓存避免频繁 IO
 *
 * 结构与 accountGroupService 相同，但使用独立的后端存储，
 * 因为 Codex 账号与 Antigravity IDE 账号是两套不同的账号体系。
 */

import { invoke } from '@tauri-apps/api/core'

let idCounter = 0;
function generateId(): string {
  return `cgrp_${Date.now()}_${++idCounter}`;
}

/** 与设置页配额自动刷新预设一致（不含 inherit） */
export const CODEX_GROUP_QUOTA_REFRESH_PRESETS = ['-1', '2', '5', '10', '15'] as const;

export const CODEX_GROUP_QUOTA_REFRESH_MIN = 1;
export const CODEX_GROUP_QUOTA_REFRESH_MAX = 999;

/**
 * 分组额度自动刷新策略（最高优先级）
 * - null: 继承平台 `codex_auto_refresh_minutes`
 * - -1: 不刷新（自动/全量跳过）
 * - 1..999: 自定义间隔（分钟）
 */
export type CodexGroupQuotaAutoRefreshMinutes = number | null;

export interface CodexAccountGroup {
  id: string;
  name: string;
  sortOrder: number;
  accountIds: string[];
  createdAt: number;
  /**
   * 分组额度自动刷新。
   * null = 继承平台；-1 = 不刷新；正整数 = 自定义分钟。
   * 兼容旧字段 `quotaRefreshEnabled: false` → -1。
   */
  quotaAutoRefreshMinutes: CodexGroupQuotaAutoRefreshMinutes;
}

/** 规范化分钟：null 继承；-1 关闭；1..999 自定义 */
export function normalizeCodexGroupQuotaAutoRefreshMinutes(
  value: unknown,
): CodexGroupQuotaAutoRefreshMinutes {
  if (value === null || value === undefined || value === '' || value === 'inherit') {
    return null;
  }
  const parsed = typeof value === 'number' ? value : Number(value);
  if (!Number.isFinite(parsed)) {
    return null;
  }
  const floored = Math.floor(parsed);
  if (floored <= -1) {
    return -1;
  }
  if (floored < CODEX_GROUP_QUOTA_REFRESH_MIN) {
    return CODEX_GROUP_QUOTA_REFRESH_MIN;
  }
  if (floored > CODEX_GROUP_QUOTA_REFRESH_MAX) {
    return CODEX_GROUP_QUOTA_REFRESH_MAX;
  }
  return floored;
}

/**
 * 从原始分组对象解析策略（兼容旧 boolean 字段）。
 * 优先级：quotaAutoRefreshMinutes > quotaRefreshEnabled(false→-1) > 继承
 */
export function resolveCodexGroupQuotaAutoRefreshMinutes(
  raw: {
    quotaAutoRefreshMinutes?: unknown;
    quotaRefreshEnabled?: unknown;
  } | null | undefined,
): CodexGroupQuotaAutoRefreshMinutes {
  if (!raw) return null;
  if (
    Object.prototype.hasOwnProperty.call(raw, 'quotaAutoRefreshMinutes') &&
    raw.quotaAutoRefreshMinutes !== undefined
  ) {
    return normalizeCodexGroupQuotaAutoRefreshMinutes(raw.quotaAutoRefreshMinutes);
  }
  // 旧版开关：false = 不刷新；true/缺省 = 继承
  if (raw.quotaRefreshEnabled === false) {
    return -1;
  }
  return null;
}

/** 分组是否允许参与自动/全量类额度刷新（-1 为否；继承/自定义为是） */
export function isCodexGroupQuotaRefreshEnabled(
  group:
    | Pick<CodexAccountGroup, 'quotaAutoRefreshMinutes'>
    | { quotaRefreshEnabled?: unknown; quotaAutoRefreshMinutes?: unknown }
    | null
    | undefined,
): boolean {
  const minutes = resolveCodexGroupQuotaAutoRefreshMinutes(group ?? undefined);
  return minutes !== -1;
}

/** 是否为「继承平台」 */
export function isCodexGroupQuotaRefreshInherit(
  group: Pick<CodexAccountGroup, 'quotaAutoRefreshMinutes'> | null | undefined,
): boolean {
  return resolveCodexGroupQuotaAutoRefreshMinutes(group) === null;
}

/**
 * 解析账号在自动刷新中的有效间隔（分钟）。
 * 分组策略最高优先级；无分组则使用平台间隔。
 * 返回 <=0 表示不自动刷新。
 */
export function resolveCodexAccountAutoRefreshMinutes(
  accountId: string,
  groups: CodexAccountGroup[],
  platformMinutes: number,
): number {
  const group = groups.find((item) => item.accountIds.includes(accountId));
  const policy = resolveCodexGroupQuotaAutoRefreshMinutes(group);
  if (policy === -1) {
    return -1;
  }
  if (typeof policy === 'number' && policy > 0) {
    return policy;
  }
  const platform = Number(platformMinutes);
  if (!Number.isFinite(platform) || platform <= 0) {
    return -1;
  }
  return Math.floor(platform);
}

function normalizeCodexGroup(
  raw: Partial<CodexAccountGroup> & {
    id?: string;
    quotaRefreshEnabled?: unknown;
  },
): CodexAccountGroup {
  return {
    id: String(raw.id ?? ''),
    name: String(raw.name ?? ''),
    sortOrder: typeof raw.sortOrder === 'number' ? raw.sortOrder : 0,
    accountIds: Array.isArray(raw.accountIds)
      ? raw.accountIds.map(String).filter(Boolean)
      : [],
    createdAt: typeof raw.createdAt === 'number' ? raw.createdAt : Date.now(),
    quotaAutoRefreshMinutes: resolveCodexGroupQuotaAutoRefreshMinutes(raw),
  };
}

// ─── 内存缓存 ───────────────────────────────────────
let cachedGroups: CodexAccountGroup[] | null = null;

function cloneGroups(groups: CodexAccountGroup[]): CodexAccountGroup[] {
  return groups.map((group) => ({
    ...normalizeCodexGroup(group),
    accountIds: [...group.accountIds],
  }));
}

async function loadGroupsFromDisk(): Promise<CodexAccountGroup[]> {
  try {
    const raw: string = await invoke('load_codex_account_groups');
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return cloneGroups(parsed.map((item) => normalizeCodexGroup(item ?? {})));
  } catch {
    return [];
  }
}

async function saveGroupsToDisk(groups: CodexAccountGroup[]): Promise<void> {
  try {
    // 落盘只写新字段；不再写旧 boolean，避免语义分叉
    const payload = groups.map((group) => ({
      id: group.id,
      name: group.name,
      sortOrder: group.sortOrder,
      accountIds: [...group.accountIds],
      createdAt: group.createdAt,
      quotaAutoRefreshMinutes: group.quotaAutoRefreshMinutes,
    }));
    await invoke('save_codex_account_groups', {
      data: JSON.stringify(payload, null, 2),
    });
  } catch (e) {
    console.error('[CodexAccountGroups] Failed to save to disk:', e);
    throw e;
  }
}

async function loadGroups(): Promise<CodexAccountGroup[]> {
  if (cachedGroups !== null) return cloneGroups(cachedGroups);
  cachedGroups = await loadGroupsFromDisk();
  return cloneGroups(cachedGroups);
}

async function saveGroups(groups: CodexAccountGroup[]): Promise<void> {
  const nextGroups = cloneGroups(groups);
  await saveGroupsToDisk(nextGroups);
  cachedGroups = nextGroups;
}

// ─── 公开 API ───────────────────────────────────────

export async function getCodexAccountGroups(): Promise<CodexAccountGroup[]> {
  const groups = await loadGroups();
  return groups.sort((a, b) => a.sortOrder - b.sortOrder);
}

export async function createCodexGroup(name: string, sortOrder?: number): Promise<CodexAccountGroup> {
  const groups = await loadGroups();
  const maxOrder = groups.length > 0 ? Math.max(...groups.map(g => g.sortOrder)) : 0;
  const group: CodexAccountGroup = {
    id: generateId(),
    name: name.trim(),
    sortOrder: sortOrder ?? maxOrder + 1,
    accountIds: [],
    createdAt: Date.now(),
    quotaAutoRefreshMinutes: null,
  };
  groups.push(group);
  await saveGroups(groups);
  return group;
}

export async function deleteCodexGroup(groupId: string): Promise<void> {
  const groups = (await loadGroups()).filter((g) => g.id !== groupId);
  await saveGroups(groups);
}

export async function renameCodexGroup(groupId: string, name: string): Promise<CodexAccountGroup | null> {
  const groups = await loadGroups();
  const group = groups.find((g) => g.id === groupId);
  if (!group) return null;
  group.name = name.trim();
  await saveGroups(groups);
  return group;
}

export async function updateCodexGroupSortOrder(groupId: string, sortOrder: number): Promise<CodexAccountGroup | null> {
  const groups = await loadGroups();
  const group = groups.find((g) => g.id === groupId);
  if (!group) return null;
  group.sortOrder = sortOrder;
  await saveGroups(groups);
  return group;
}

/** 设置分组额度自动刷新策略（null=继承，-1=不刷新，>0=自定义分钟） */
export async function setCodexGroupQuotaAutoRefreshMinutes(
  groupId: string,
  minutes: CodexGroupQuotaAutoRefreshMinutes,
): Promise<CodexAccountGroup | null> {
  const groups = await loadGroups();
  const group = groups.find((g) => g.id === groupId);
  if (!group) return null;
  group.quotaAutoRefreshMinutes = normalizeCodexGroupQuotaAutoRefreshMinutes(minutes);
  await saveGroups(groups);
  // 触发自动刷新调度重建（与设置页变更一致）
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent('config-updated'));
  }
  return group;
}

/**
 * @deprecated 使用 setCodexGroupQuotaAutoRefreshMinutes
 * 保留兼容：enabled=false → -1，true → 继承
 */
export async function setCodexGroupQuotaRefreshEnabled(
  groupId: string,
  enabled: boolean,
): Promise<CodexAccountGroup | null> {
  return setCodexGroupQuotaAutoRefreshMinutes(groupId, enabled ? null : -1);
}

/**
 * 返回「所属分组关闭了额度刷新」的账号 ID 集合。
 * 未分组账号不在集合中（允许刷新）。
 */
export async function getCodexQuotaRefreshDisabledAccountIds(): Promise<Set<string>> {
  const groups = await loadGroups();
  const disabled = new Set<string>();
  for (const group of groups) {
    if (isCodexGroupQuotaRefreshEnabled(group)) continue;
    for (const accountId of group.accountIds) {
      disabled.add(accountId);
    }
  }
  return disabled;
}

/**
 * 按自定义间隔聚合账号（仅 >0 的自定义分钟）。
 * key = 分钟，value = 账号 ID 列表。
 */
export async function getCodexCustomQuotaRefreshAccountIdsByMinutes(): Promise<
  Map<number, string[]>
> {
  const groups = await loadGroups();
  const map = new Map<number, string[]>();
  for (const group of groups) {
    const minutes = resolveCodexGroupQuotaAutoRefreshMinutes(group);
    if (typeof minutes !== 'number' || minutes <= 0) continue;
    const list = map.get(minutes) ?? [];
    for (const accountId of group.accountIds) {
      if (accountId && !list.includes(accountId)) {
        list.push(accountId);
      }
    }
    map.set(minutes, list);
  }
  return map;
}

/** 继承平台策略的账号 ID（含未分组） */
export async function getCodexInheritPlatformQuotaRefreshAccountIds(
  allAccountIds: string[],
): Promise<string[]> {
  const groups = await loadGroups();
  const grouped = new Map<string, CodexAccountGroup>();
  for (const group of groups) {
    for (const accountId of group.accountIds) {
      grouped.set(accountId, group);
    }
  }
  return allAccountIds.filter((accountId) => {
    const group = grouped.get(accountId);
    if (!group) return true;
    return resolveCodexGroupQuotaAutoRefreshMinutes(group) === null;
  });
}

export async function addAccountsToCodexGroup(groupId: string, accountIds: string[]): Promise<CodexAccountGroup | null> {
  return assignAccountsToCodexGroup(groupId, accountIds);
}

export async function assignAccountsToCodexGroup(groupId: string, accountIds: string[]): Promise<CodexAccountGroup | null> {
  const groups = await loadGroups();
  const group = groups.find((g) => g.id === groupId);
  if (!group) return null;
  const targetIds = new Set(accountIds);

  // 从其他分组中移除
  for (const currentGroup of groups) {
    if (currentGroup.id === groupId) continue;
    currentGroup.accountIds = currentGroup.accountIds.filter((id) => !targetIds.has(id));
  }

  // 添加到目标分组
  const existing = new Set(group.accountIds);
  for (const id of accountIds) {
    if (!existing.has(id)) {
      group.accountIds.push(id);
      existing.add(id);
    }
  }
  await saveGroups(groups);
  return group;
}

export async function removeAccountsFromCodexGroup(groupId: string, accountIds: string[]): Promise<CodexAccountGroup | null> {
  const groups = await loadGroups();
  const group = groups.find((g) => g.id === groupId);
  if (!group) return null;
  const toRemove = new Set(accountIds);
  group.accountIds = group.accountIds.filter((id) => !toRemove.has(id));
  await saveGroups(groups);
  return group;
}

/** 只移除明确已删除的账号，禁止用空列表把整组清掉。 */
export async function removeAccountIdsFromAllCodexGroups(
  accountIds: string[],
): Promise<void> {
  const toRemove = new Set(accountIds.map((id) => id.trim()).filter(Boolean));
  if (toRemove.size === 0) return;
  const groups = await loadGroups();
  let changed = false;
  for (const group of groups) {
    const next = group.accountIds.filter((id) => !toRemove.has(id));
    if (next.length !== group.accountIds.length) {
      group.accountIds = next;
      changed = true;
    }
  }
  if (changed) await saveGroups(groups);
}

/** 清理不存在的账号ID（仅在确认当前列表完整时使用） */
export async function cleanupDeletedCodexAccounts(existingAccountIds: Set<string>): Promise<void> {
  if (existingAccountIds.size === 0) return;
  const groups = await loadGroups();
  let changed = false;
  for (const group of groups) {
    const before = group.accountIds.length;
    group.accountIds = group.accountIds.filter((id) => existingAccountIds.has(id));
    if (group.accountIds.length !== before) changed = true;
  }
  if (changed) await saveGroups(groups);
}

/** 将账号从一个分组移动到另一个分组 */
export async function moveAccountsBetweenCodexGroups(
  fromGroupId: string,
  toGroupId: string,
  accountIds: string[]
): Promise<void> {
  if (fromGroupId === toGroupId) return;
  await assignAccountsToCodexGroup(toGroupId, accountIds);
}

/** 使缓存失效，下次 getCodexAccountGroups 时重新从磁盘读取 */
export function invalidateCodexGroupCache(): void {
  cachedGroups = null;
}
