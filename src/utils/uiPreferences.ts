import { invoke } from '@tauri-apps/api/core';

const PLATFORM_LAYOUT_KEY = 'agtools.platform_layout.v1';
const TIMEOUT_MS = 3000;
const STATUS_EVENT = 'agtools:platform-layout-persistence-changed';

export interface UiPreferencesSnapshot {
  values: Record<string, string>;
  revisions?: Record<string, number>;
}

let hydrated = false;
let hydrationPromise: Promise<void> | null = null;
let flushing: Promise<void> | null = null;
let pending: string | null = null;
let committed: string | null = null;
let visible: string | null | undefined;
let diskRevision = 0;
let lastRevision = 0;
let persistenceError: string | null = null;

function reportError(error: unknown): void {
  persistenceError = error == null ? null : String(error);
  if (error != null) console.warn('[uiPreferences]', error);
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(STATUS_EVENT));
}

export function getPlatformLayoutPersistenceError(): string | null {
  return persistenceError;
}

function layout(value: string | null): Record<string, unknown> | null {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value);
    if (parsed && !Array.isArray(parsed) &&
      (Array.isArray(parsed.orderedEntryIds) || Array.isArray(parsed.orderedPlatformIds)
        || Array.isArray(parsed.platformGroups))) {
      return parsed;
    }
  } catch { /* invalid caches cannot replace durable preferences */ }
  return null;
}

function revision(value: string | null): number {
  const n = layout(value)?._layoutUpdatedAt;
  return typeof n === 'number' && Number.isSafeInteger(n) && n >= 0 ? n : 0;
}

function withRevision(value: string): string {
  lastRevision = Math.max(Date.now(), lastRevision + 1, revision(value) + 1);
  return JSON.stringify({ ...layout(value), _layoutUpdatedAt: lastRevision });
}

function readLocal(): string | null {
  try { return localStorage.getItem(PLATFORM_LAYOUT_KEY); }
  catch (error) { reportError(error); return null; }
}

function visibleLayout(): string | null {
  if (visible === undefined) visible = readLocal();
  return visible;
}

export function readPlatformLayout(): string | null {
  const value = visibleLayout();
  return value === 'null' ? null : value;
}

/** Use only after hydration; an existing layout includes explicit empty selections. */
export function hasSavedPlatformLayout(): boolean {
  const value = visibleLayout();
  return value === 'null' || layout(value) != null;
}

function notifyLayoutChanged(): void {
  if (typeof window !== 'undefined') window.dispatchEvent(new Event('agtools:platform-layout-hydrated'));
}

export function applyPlatformLayoutSnapshot(snapshot: UiPreferencesSnapshot): void {
  if (!hydrated || flushing || pending !== null) return;
  const value = snapshot.values[PLATFORM_LAYOUT_KEY];
  const revision = snapshot.revisions?.[PLATFORM_LAYOUT_KEY];
  if (revision === undefined || revision <= diskRevision || (value !== 'null' && !layout(value ?? null))) return;
  const changed = visible !== value;
  visible = committed = value;
  diskRevision = revision;
  if (changed) notifyLayoutChanged();
}

// At most one save is in flight. A timeout reports failure but does not launch
// another overlapping writer: an IPC timeout cannot cancel the old disk write.
function flush(): Promise<void> {
  if (flushing) return flushing;
  if (!hydrated || pending == null) return Promise.resolve();
  flushing = flushPending().finally(() => { flushing = null; });
  return flushing;
}

async function flushPending(): Promise<void> {
  while (pending != null) {
    const value = pending;
    pending = null;
    const timer = setTimeout(() => reportError(new Error('UI preferences save timeout')), TIMEOUT_MS);
    try {
      const saved = await invoke<UiPreferencesSnapshot>('update_ui_preference', {
        key: PLATFORM_LAYOUT_KEY, value, expectedRevision: diskRevision,
      });
      committed = value;
      diskRevision = saved.revisions?.[PLATFORM_LAYOUT_KEY] ?? diskRevision;
      reportError(null);
    } catch (error) {
      pending ??= value;
      reportError(error);
      break;
    } finally { clearTimeout(timer); }
  }
}

export function persistPlatformLayout(value: string): void {
  if (!layout(value)) {
    reportError(new Error('Invalid platform layout'));
    return;
  }
  reportError(null);
  lastRevision = Math.max(lastRevision, revision(visibleLayout()));
  pending = withRevision(value);
  visible = pending;
  if (hydrated) void flush();
  else void hydrateUiPreferences();
}

export function hydrateUiPreferences(): Promise<void> {
  if (hydrationPromise) return hydrationPromise;
  if (hydrated) return flush();
  const initial = visibleLayout();
  hydrationPromise = (async () => {
    try {
      const snapshot = await loadLayoutSnapshot();
      const raw = snapshot?.values?.[PLATFORM_LAYOUT_KEY] ?? null;
      if (raw != null && raw !== 'null' && !layout(raw)) throw new Error('Invalid saved platform layout');
      committed = raw;
      diskRevision = snapshot?.revisions?.[PLATFORM_LAYOUT_KEY] ?? 0;
      const edited = pending !== null;
      const local = pending ?? initial;
      const usableLocal = layout(local) ? local : null;
      lastRevision = Math.max(lastRevision, revision(raw), revision(usableLocal));
      // An older dual-write build can leave a newer cache if its disk save was interrupted.
      const legacyWins = usableLocal != null && raw !== 'null'
        && (raw == null || revision(usableLocal) > revision(raw));
      const chosen = edited || legacyWins ? usableLocal : raw;
      const next = chosen != null && chosen !== raw && (revision(chosen) === 0 || edited)
        ? withRevision(chosen) : chosen;
      visible = next;
      pending = next !== raw ? next : null;
      hydrated = true;
      if (!edited && next !== initial) notifyLayoutChanged();
      reportError(null);
      await flush();
    } catch (error) {
      // Load failure is NOT an empty file. Do not write a cache over unknown
      // durable data; retain pending changes and let an explicit retry reconcile.
      reportError(error);
    } finally {
      hydrationPromise = null;
    }
  })();
  return hydrationPromise;
}

async function withLayoutDeadline<T>(task: Promise<T>, message: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([task, new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), TIMEOUT_MS);
    })]);
  } finally { if (timer != null) clearTimeout(timer); }
}

async function waitForLayoutSave(): Promise<void> {
  await withLayoutDeadline(hydrateUiPreferences(), 'UI preferences save timeout');
  if (persistenceError && (!hydrated || pending !== null)) throw new Error(persistenceError);
}

function loadLayoutSnapshot(): Promise<UiPreferencesSnapshot> {
  return withLayoutDeadline(invoke<UiPreferencesSnapshot>('load_ui_preferences'), 'UI preferences load timeout');
}

export async function exportPlatformLayout(): Promise<unknown> {
  await waitForLayoutSave();
  const snapshot = await loadLayoutSnapshot();
  const value = snapshot.values[PLATFORM_LAYOUT_KEY];
  return value === undefined ? undefined : JSON.parse(validateImportedPlatformLayout(value));
}

export function validateImportedPlatformLayout(value: unknown): string {
  const serialized = typeof value === 'string' ? value : JSON.stringify(value);
  if (serialized === 'null' || layout(serialized)) return serialized;
  throw new Error('invalid_platform_layout');
}

export async function restorePlatformLayout(serialized: string): Promise<void> {
  validateImportedPlatformLayout(serialized);
  await waitForLayoutSave();
  const value = serialized === 'null' ? serialized : withRevision(serialized);
  pending = value;
  const task = flush().then(async () => {
    if (committed !== value) {
      const failure = persistenceError ?? 'Platform layout restore was superseded';
      if (pending === value) pending = null;
      if (pending == null && failure.includes('ui_preference_revision_conflict')) {
        try {
          applyPlatformLayoutSnapshot(await loadLayoutSnapshot());
        } catch (error) {
          console.warn('[uiPreferences]', error);
        }
      }
      throw new Error(failure);
    }
    if (pending == null) {
      visible = value;
      notifyLayoutChanged();
    }
  });
  await withLayoutDeadline(task, 'UI preferences save timeout');
}

export async function retryPlatformLayoutPersistence(): Promise<void> {
  if (flushing) {
    reportError(new Error('UI preferences save is still pending'));
    return;
  }
  reportError(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([
      !hydrated ? hydrateUiPreferences() : flush(),
      new Promise<void>((resolve) => {
        timer = setTimeout(() => {
          reportError(new Error('UI preferences retry timeout'));
          resolve();
        }, TIMEOUT_MS);
      }),
    ]);
  } finally { if (timer != null) clearTimeout(timer); }
}
