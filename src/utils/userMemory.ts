import { invoke } from "@tauri-apps/api/core";

export const USER_MEMORY_FLAGS = {
  gatewayGuide: "codex.gateway_guide",
  riskNotice: "codex.local_access_risk_notice",
  classicSwitchPrompt: "side_nav.hide_classic_switch_prompt",
} as const;

export const USER_MEMORY_LISTS = {
  antigravityCustomSort: "antigravity.accounts.custom_sort",
  codexCustomSort: "codex.accounts.custom_sort",
  codexTagGroupOrder: "codex.accounts.tag_group_order",
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
const listPreviews = new Map<string, string[]>();
const listWrites = new Map<string, Promise<void>>();
const listeners = new Set<() => void>();
let hydrated = false;
let hydratePromise: Promise<void> | null = null;

function notifyUserMemoryListeners(): void {
  listeners.forEach((listener) => listener());
}

function saveUserMemoryList(id: string, items: string[]): Promise<void> {
  // Keep a pending local save from landing after a restored order.
  const pending = (listWrites.get(id) ?? Promise.resolve())
    .catch(() => undefined)
    .then(() => invoke<void>("save_user_memory_list", { id, items }));
  listWrites.set(id, pending);
  const cleanup = () => {
    if (listWrites.get(id) === pending) listWrites.delete(id);
  };
  void pending.then(cleanup, cleanup);
  return pending;
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

export function isUserMemoryDismissed(id: string): boolean {
  return diskDismissed.get(id) ?? (!hydrated && readLocalStorageFlag(id));
}

export function readUserMemoryList(id: string): string[] {
  const cached = listPreviews.get(id) ?? listCache.get(id);
  return cached ? [...cached] : hydrated ? [] : readLocalStorageList(id);
}

export async function hydrateUserMemory(): Promise<UserMemorySnapshot> {
  if (!hydrated && !hydratePromise) {
    hydratePromise = (async () => {
      const legacy: UserMemorySnapshot = { dismissed: {}, lists: {} };
      for (const id of Object.values(USER_MEMORY_FLAGS)) {
        if (readLocalStorageFlag(id)) legacy.dismissed[id] = true;
      }
      for (const id of Object.values(USER_MEMORY_LISTS)) {
        const items = readLocalStorageList(id);
        if (items.length > 0) legacy.lists[id] = items;
      }
      const snapshot = await invoke<UserMemorySnapshot>("load_user_memory", { legacy });
      Object.entries(snapshot.dismissed ?? {}).forEach(([id, dismissed]) => diskDismissed.set(id, dismissed));
      Object.entries(snapshot.lists ?? {}).forEach(([id, items]) => listCache.set(id, mergeIdLists(items)));
      hydrated = true;
      notifyUserMemoryListeners();
    })().finally(() => { hydratePromise = null; });
  }
  await hydratePromise;
  return {
    dismissed: Object.fromEntries(diskDismissed),
    lists: Object.fromEntries(Array.from(listCache, ([id, items]) => [id, [...items]])),
  };
}

export async function markUserMemoryDismissed(id: string): Promise<void> {
  try {
    await hydrateUserMemory();
    if (diskDismissed.get(id) === true) return;
    await invoke("mark_user_memory_dismissed", { id });
    diskDismissed.set(id, true);
  } catch (error) {
    console.warn("[User memory]", error);
  }
}

export function persistUserMemoryList(id: string, items: string[]): void {
  const next = mergeIdLists(items);
  // A list can be empty while its accounts are still loading.
  if (next.length === 0) return;
  const previous = listPreviews.get(id) ?? listCache.get(id);
  if (previous?.length === next.length && previous.every((value, index) => value === next[index])) return;
  listPreviews.set(id, next);
  void (async () => {
    await hydrateUserMemory();
    await saveUserMemoryList(id, next);
    listCache.set(id, next);
  })().catch(error => console.warn("[User memory]", error)).finally(() => {
    if (listPreviews.get(id) === next) listPreviews.delete(id);
    notifyUserMemoryListeners();
  });
}

/** Restore only after durable storage succeeds, then refresh mounted readers. */
export async function restoreUserMemoryList(id: string, items: string[]): Promise<void> {
  await hydrateUserMemory();
  const next = mergeIdLists(items);
  if (next.length === 0) return;
  await saveUserMemoryList(id, next);
  listCache.set(id, next);
  notifyUserMemoryListeners();
}
