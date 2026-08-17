import { invoke } from "@tauri-apps/api/core";

export const USER_MEMORY_FLAGS = {
  gatewayGuide: "codex.gateway_guide",
  riskNotice: "codex.local_access_risk_notice",
  classicSwitchPrompt: "side_nav.hide_classic_switch_prompt",
} as const;

export const USER_MEMORY_LISTS = {
  antigravityCustomSort: "antigravity.accounts.custom_sort",
  codexCustomSort: "codex.accounts.custom_sort",
} as const;

const LOCAL_STORAGE_KEYS: Record<string, string[]> = {
  [USER_MEMORY_FLAGS.gatewayGuide]: [
    "agtools.codex.api_service.gateway_guide.dismissed.v1",
  ],
  [USER_MEMORY_FLAGS.riskNotice]: [
    "agtools.codex.local_access.risk_notice.dismissed.v2",
    "agtools.codex.local_access.risk_notice.dismissed.v1",
  ],
  [USER_MEMORY_FLAGS.classicSwitchPrompt]: [
    "agtools.side_nav.hide_classic_switch_prompt.v1",
  ],
};

const LOCAL_STORAGE_LIST_KEYS: Record<string, string[]> = {
  [USER_MEMORY_LISTS.antigravityCustomSort]: [
    "agtools.antigravity.accounts.custom_sort_order.v1",
  ],
  [USER_MEMORY_LISTS.codexCustomSort]: [
    "agtools.codex.accounts.custom_sort_order.v1",
  ],
};

export interface UserMemorySnapshot {
  dismissed: Record<string, boolean>;
  lists: Record<string, string[]>;
}

const diskDismissed = new Map<string, boolean>();
const listCache = new Map<string, string[]>();
const listeners = new Set<() => void>();
let hydrated = false;
let hydratePromise: Promise<void> | null = null;

function notifyUserMemoryListeners(): void {
  listeners.forEach((listener) => listener());
}

export function subscribeUserMemory(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function mergeIdLists(
  preferred: string[],
  fallback: string[] = [],
): string[] {
  const seen = new Set<string>();
  const next: string[] = [];
  for (const value of [...preferred, ...fallback]) {
    const id = value.trim();
    if (!id || seen.has(id)) continue;
    seen.add(id);
    next.push(id);
  }
  return next;
}

export function mergeIdListsPreferExisting(
  preferred: string[],
  extra: string[] = [],
): string[] {
  const merged = mergeIdLists(
    preferred.length > 0 ? preferred : extra,
    extra,
  );
  if (
    merged.length === preferred.length &&
    merged.every((id, index) => id === preferred[index])
  ) {
    return preferred;
  }
  return merged;
}

function readLocalStorageFlag(id: string): boolean {
  try {
    return (LOCAL_STORAGE_KEYS[id] ?? []).some(
      (key) => globalThis.localStorage?.getItem(key) === "1",
    );
  } catch {
    return false;
  }
}

function writeLocalStorageFlag(id: string): void {
  try {
    for (const key of LOCAL_STORAGE_KEYS[id] ?? []) {
      globalThis.localStorage?.setItem(key, "1");
    }
  } catch {
    // ignore quota / private-mode failures
  }
}

function parseStoredIdList(raw: string | null): string[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return mergeIdLists(
      parsed.filter((item): item is string => typeof item === "string"),
    );
  } catch {
    return [];
  }
}

function readLocalStorageList(id: string): string[] {
  try {
    for (const key of LOCAL_STORAGE_LIST_KEYS[id] ?? []) {
      const parsed = parseStoredIdList(
        globalThis.localStorage?.getItem(key) ?? null,
      );
      if (parsed.length > 0) return parsed;
    }
  } catch {
    // ignore quota / private-mode failures
  }
  return [];
}

function writeLocalStorageList(id: string, items: string[]): void {
  const raw = JSON.stringify(items);
  try {
    for (const key of LOCAL_STORAGE_LIST_KEYS[id] ?? []) {
      globalThis.localStorage?.setItem(key, raw);
    }
  } catch {
    // ignore quota / private-mode failures
  }
}

export function isUserMemoryDismissed(id: string): boolean {
  if (diskDismissed.get(id) === true) return true;
  return readLocalStorageFlag(id);
}

export function readUserMemoryList(id: string): string[] {
  const cached = listCache.get(id);
  if (cached && cached.length > 0) return [...cached];
  return readLocalStorageList(id);
}

export async function hydrateUserMemory(): Promise<UserMemorySnapshot> {
  if (!hydratePromise) {
    hydratePromise = (async () => {
      try {
        const snapshot = await invoke<UserMemorySnapshot>("load_user_memory");
        Object.entries(snapshot.dismissed ?? {}).forEach(([key, value]) => {
          if (value) diskDismissed.set(key, true);
        });
        Object.values(USER_MEMORY_FLAGS).forEach((flag) => {
          if (readLocalStorageFlag(flag) && diskDismissed.get(flag) !== true) {
            diskDismissed.set(flag, true);
            void invoke("mark_user_memory_dismissed", { id: flag }).catch(
              () => undefined,
            );
          }
          if (diskDismissed.get(flag) === true) {
            writeLocalStorageFlag(flag);
          }
        });
        Object.values(USER_MEMORY_LISTS).forEach((listId) => {
          const diskItems = snapshot.lists?.[listId] ?? [];
          const localItems = readLocalStorageList(listId);
          const merged = mergeIdLists(
            diskItems.length > 0 ? diskItems : localItems,
            localItems,
          );
          if (merged.length === 0) return;
          listCache.set(listId, merged);
          writeLocalStorageList(listId, merged);
          const sameAsDisk =
            diskItems.length === merged.length &&
            diskItems.every((item, index) => item === merged[index]);
          if (!sameAsDisk) {
            void invoke("save_user_memory_list", {
              id: listId,
              items: merged,
            }).catch(() => undefined);
          }
        });
      } catch {
        Object.values(USER_MEMORY_FLAGS).forEach((flag) => {
          if (readLocalStorageFlag(flag)) diskDismissed.set(flag, true);
        });
        Object.values(USER_MEMORY_LISTS).forEach((listId) => {
          const localItems = readLocalStorageList(listId);
          if (localItems.length > 0) listCache.set(listId, localItems);
        });
      } finally {
        hydrated = true;
        notifyUserMemoryListeners();
      }
    })();
  }
  await hydratePromise;
  return {
    dismissed: Object.fromEntries(diskDismissed.entries()),
    lists: Object.fromEntries(
      Array.from(listCache.entries()).map(([key, value]) => [key, [...value]]),
    ),
  };
}

export function hasHydratedUserMemory(): boolean {
  return hydrated;
}

export async function markUserMemoryDismissed(id: string): Promise<void> {
  diskDismissed.set(id, true);
  writeLocalStorageFlag(id);
  try {
    await invoke("mark_user_memory_dismissed", { id });
  } catch {
    // localStorage already written; disk can catch up next launch
  }
}

export function persistUserMemoryList(id: string, items: string[]): void {
  const next = mergeIdLists(items);
  writeLocalStorageList(id, next);
  if (next.length === 0) {
    return;
  }
  // 启动尚未读完磁盘时，只写 localStorage，避免把未就绪的账号顺序覆盖落盘记忆。
  if (!hydrated) {
    return;
  }
  listCache.set(id, next);
  void invoke("save_user_memory_list", { id, items: next }).catch(
    () => undefined,
  );
}
