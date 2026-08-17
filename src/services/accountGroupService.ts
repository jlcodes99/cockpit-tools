/**
 * 账号分组服务
 * 数据通过 Tauri 命令持久化到磁盘 (~/.antigravity_cockpit/account_groups.json)
 * 内存中维护一份缓存避免频繁 IO
 */

import { invoke } from '@tauri-apps/api/core'

const LEGACY_STORAGE_KEY = 'agtools.account_groups';

let idCounter = 0;
function generateId(): string {
  return `grp_${Date.now()}_${++idCounter}`;
}

export interface AccountGroup {
  id: string;
  name: string;
  accountIds: string[];
  createdAt: number;
}

// ─── 内存缓存 ───────────────────────────────────────
let cachedGroups: AccountGroup[] | null = null;

// Serialize read-modify-write operations so concurrent UI actions cannot
// overwrite a newer group membership snapshot with a stale one.
let groupsOperation: Promise<unknown> = Promise.resolve();

function enqueue<T>(operation: () => Promise<T>): Promise<T> {
  const next = groupsOperation.then(operation, operation);
  groupsOperation = next.then(
    () => undefined,
    () => undefined,
  );
  return next;
}

function cloneGroups(groups: AccountGroup[]): AccountGroup[] {
  return groups.map((group) => ({
    ...group,
    accountIds: [...group.accountIds],
  }));
}

function parseGroups(raw: string): AccountGroup[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    throw new Error(`[AccountGroups] Invalid JSON: ${String(error)}`);
  }

  if (!Array.isArray(parsed)) {
    throw new Error('[AccountGroups] Stored data must be a JSON array');
  }

  const groups = parsed.map((value, index) => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      throw new Error(`[AccountGroups] Invalid group at index ${index}`);
    }

    const group = value as Record<string, unknown>;
    if (typeof group.id !== 'string' || !group.id.trim()) {
      throw new Error(`[AccountGroups] Group ${index} has an invalid id`);
    }
    if (typeof group.name !== 'string') {
      throw new Error(`[AccountGroups] Group ${index} has an invalid name`);
    }
    if (!Array.isArray(group.accountIds) || group.accountIds.some((id) => typeof id !== 'string')) {
      throw new Error(`[AccountGroups] Group ${index} has invalid account ids`);
    }

    return {
      ...group,
      id: group.id,
      name: group.name,
      accountIds: [...group.accountIds],
      // Keep older files without createdAt readable without treating them as
      // an empty data set.
      createdAt: typeof group.createdAt === 'number' ? group.createdAt : 0,
    } as AccountGroup;
  });

  return cloneGroups(groups);
}

async function loadGroupsFromDisk(): Promise<AccountGroup[]> {
  try {
    const raw: string = await invoke('load_account_groups');
    return parseGroups(raw);
  } catch (error) {
    if (error instanceof Error && error.message.startsWith('[AccountGroups]')) {
      throw error;
    }
    throw new Error(`[AccountGroups] Failed to load groups: ${String(error)}`);
  }
}

async function saveGroupsToDisk(groups: AccountGroup[]): Promise<void> {
  try {
    await invoke('save_account_groups', { data: JSON.stringify(groups, null, 2) });
  } catch (e) {
    console.error('[AccountGroups] Failed to save to disk:', e);
    throw e;
  }
}

/** 迁移 localStorage 数据到磁盘（仅首次） */
async function migrateLegacyData(): Promise<void> {
  try {
    const raw = localStorage.getItem(LEGACY_STORAGE_KEY);
    if (!raw) return;
    const legacy = JSON.parse(raw);
    if (!Array.isArray(legacy) || legacy.length === 0) return;

    // 检查磁盘上是否已有数据
    const diskData = await loadGroupsFromDisk();
    if (diskData.length > 0) {
      // 磁盘已有数据，清理 localStorage
      localStorage.removeItem(LEGACY_STORAGE_KEY);
      return;
    }

    // 迁移到磁盘
    await saveGroupsToDisk(legacy);
    localStorage.removeItem(LEGACY_STORAGE_KEY);
    console.log('[AccountGroups] Migrated', legacy.length, 'groups from localStorage to disk');
  } catch {
    // ignore migration errors
  }
}

async function loadGroups(): Promise<AccountGroup[]> {
  if (cachedGroups !== null) return cloneGroups(cachedGroups);

  // 首次加载时尝试迁移 localStorage 数据
  await migrateLegacyData();

  cachedGroups = await loadGroupsFromDisk();
  return cloneGroups(cachedGroups);
}

async function saveGroups(groups: AccountGroup[]): Promise<void> {
  const nextGroups = cloneGroups(groups);
  await saveGroupsToDisk(nextGroups);
  cachedGroups = nextGroups;
}

// ─── 公开 API（全部改为 async） ───────────────────────

export function getAccountGroups(): Promise<AccountGroup[]> {
  return enqueue(() => loadGroups());
}

export function createGroup(name: string): Promise<AccountGroup> {
  return enqueue(async () => {
    const groups = await loadGroups();
    const group: AccountGroup = {
      id: generateId(),
      name: name.trim(),
      accountIds: [],
      createdAt: Date.now(),
    };
    groups.push(group);
    await saveGroups(groups);
    return group;
  });
}

export function deleteGroup(groupId: string): Promise<void> {
  return enqueue(async () => {
    const groups = (await loadGroups()).filter((g) => g.id !== groupId);
    await saveGroups(groups);
  });
}

export function renameGroup(groupId: string, name: string): Promise<AccountGroup | null> {
  return enqueue(async () => {
    const groups = await loadGroups();
    const group = groups.find((g) => g.id === groupId);
    if (!group) return null;
    group.name = name.trim();
    await saveGroups(groups);
    return group;
  });
}

export function addAccountsToGroup(groupId: string, accountIds: string[]): Promise<AccountGroup | null> {
  return enqueue(() => assignAccountsToGroupInternal(groupId, accountIds));
}

async function assignAccountsToGroupInternal(groupId: string, accountIds: string[]): Promise<AccountGroup | null> {
  const groups = await loadGroups();
  const group = groups.find((g) => g.id === groupId);
  if (!group) return null;
  const targetIds = new Set(accountIds);

  for (const currentGroup of groups) {
    if (currentGroup.id === groupId) continue;
    currentGroup.accountIds = currentGroup.accountIds.filter((id) => !targetIds.has(id));
  }

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

export function assignAccountsToGroup(groupId: string, accountIds: string[]): Promise<AccountGroup | null> {
  return enqueue(() => assignAccountsToGroupInternal(groupId, accountIds));
}

export function removeAccountsFromGroup(groupId: string, accountIds: string[]): Promise<AccountGroup | null> {
  return enqueue(async () => {
    const groups = await loadGroups();
    const group = groups.find((g) => g.id === groupId);
    if (!group) return null;
    const toRemove = new Set(accountIds);
    group.accountIds = group.accountIds.filter((id) => !toRemove.has(id));
    await saveGroups(groups);
    return group;
  });
}

/** 只移除明确已删除的账号，禁止用空列表把整组清掉。 */
export function removeAccountIdsFromAllGroups(accountIds: string[]): Promise<void> {
  return enqueue(async () => {
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
  });
}

/** 清理不存在的账号ID（仅在确认当前列表完整时使用） */
export function cleanupDeletedAccounts(existingAccountIds: Set<string>): Promise<void> {
  return enqueue(async () => {
    if (existingAccountIds.size === 0) return;
    const groups = await loadGroups();
    let changed = false;
    for (const group of groups) {
      const before = group.accountIds.length;
      group.accountIds = group.accountIds.filter((id) => existingAccountIds.has(id));
      if (group.accountIds.length !== before) changed = true;
    }
    if (changed) await saveGroups(groups);
  });
}

/** 将账号从一个分组移动到另一个分组 */
export function moveAccountsBetweenGroups(
  fromGroupId: string,
  toGroupId: string,
  accountIds: string[]
): Promise<void> {
  return enqueue(async () => {
    if (fromGroupId === toGroupId) return;
    await assignAccountsToGroupInternal(toGroupId, accountIds);
  });
}

/** 使缓存失效，下次 getAccountGroups 时重新从磁盘读取 */
export function invalidateCache(): void {
  cachedGroups = null;
}
