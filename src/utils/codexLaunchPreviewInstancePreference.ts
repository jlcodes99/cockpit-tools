const CODEX_LAUNCH_PREVIEW_LAST_INSTANCE_STORAGE_KEY =
  "agtools.codex.launch_preview.last_instance.v1";

export const CODEX_LAUNCH_PREVIEW_API_SERVICE_CARD_KEY = "__api_service__";

type PreferenceStorage = Pick<Storage, "getItem" | "setItem">;

function readStorage(storage?: PreferenceStorage): Storage | PreferenceStorage | null {
  if (storage) return storage;
  try {
    return localStorage;
  } catch {
    return null;
  }
}

function normalizeCardKey(cardKey: string): string {
  return cardKey.trim();
}

function parseStoredMap(raw: string | null): Record<string, string> {
  if (!raw) return {};
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }
    const next: Record<string, string> = {};
    for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof key !== "string" || !key.trim()) continue;
      if (typeof value !== "string" || !value.trim()) continue;
      next[key.trim()] = value.trim();
    }
    return next;
  } catch {
    return {};
  }
}

function readStoredMap(storage?: PreferenceStorage): Record<string, string> {
  const store = readStorage(storage);
  if (!store) return {};
  try {
    return parseStoredMap(store.getItem(CODEX_LAUNCH_PREVIEW_LAST_INSTANCE_STORAGE_KEY));
  } catch {
    return {};
  }
}

export function resolveCodexLaunchPreviewLastInstanceId(
  savedInstanceId: string | null | undefined,
  availableInstanceIds: readonly string[],
  fallbackInstanceId: string,
): string {
  const saved = savedInstanceId?.trim() || "";
  if (saved && availableInstanceIds.includes(saved)) {
    return saved;
  }
  if (availableInstanceIds.includes(fallbackInstanceId)) {
    return fallbackInstanceId;
  }
  return availableInstanceIds[0] ?? fallbackInstanceId;
}

export function readCodexLaunchPreviewLastInstanceId(
  cardKey: string,
  availableInstanceIds: readonly string[],
  fallbackInstanceId: string,
  storage?: PreferenceStorage,
): string {
  const key = normalizeCardKey(cardKey);
  const saved = key ? readStoredMap(storage)[key] : undefined;
  return resolveCodexLaunchPreviewLastInstanceId(
    saved,
    availableInstanceIds,
    fallbackInstanceId,
  );
}

export function persistCodexLaunchPreviewLastInstanceId(
  cardKey: string,
  instanceId: string,
  storage?: PreferenceStorage,
): boolean {
  const key = normalizeCardKey(cardKey);
  const value = instanceId.trim();
  if (!key || !value) return false;
  const store = readStorage(storage);
  if (!store) return false;
  try {
    const next = {
      ...readStoredMap(storage),
      [key]: value,
    };
    store.setItem(
      CODEX_LAUNCH_PREVIEW_LAST_INSTANCE_STORAGE_KEY,
      JSON.stringify(next),
    );
    return true;
  } catch {
    return false;
  }
}
