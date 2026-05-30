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

export interface CodexAccountGroup {
  id: string;
  name: string;
  sortOrder: number;
  accountIds: string[];
  createdAt: number;
}

// ─── 内存缓存 ───────────────────────────────────────
let cachedGroups: CodexAccountGroup[] | null = null;

function cloneGroups(groups: CodexAccountGroup[]): CodexAccountGroup[] {
  return groups.map((group) => ({
    ...group,
    accountIds: [...group.accountIds],
  }));
}

async function loadGroupsFromDisk(): Promise<CodexAccountGroup[]> {
  try {
    const raw: string = await invoke('load_codex_account_groups');
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? cloneGroups(parsed) : [];
  } catch {
    return [];
  }
}

async function saveGroupsToDisk(groups: CodexAccountGroup[]): Promise<void> {
  try {
    await invoke('save_codex_account_groups', { data: JSON.stringify(groups, null, 2) });
  } catch (e) {
    console.error('[CodexAccountGroups] Failed to save to disk:', e);
  }
}

async function loadGroups(): Promise<CodexAccountGroup[]> {
  if (cachedGroups !== null) return cloneGroups(cachedGroups);
  cachedGroups = await loadGroupsFromDisk();
  return cloneGroups(cachedGroups);
}

async function saveGroups(groups: CodexAccountGroup[]): Promise<void> {
  const nextGroups = cloneGroups(groups);
  cachedGroups = nextGroups;
  await saveGroupsToDisk(nextGroups);
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

/** 清理不存在的账号ID（当账号被删除时调用） */
export async function cleanupDeletedCodexAccounts(existingAccountIds: Set<string>): Promise<void> {
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
