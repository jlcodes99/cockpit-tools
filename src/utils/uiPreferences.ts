import { invoke } from '@tauri-apps/api/core';

const PLATFORM_LAYOUT_KEY = 'agtools.platform_layout.v1';
const TIMEOUT_MS = 3000;
const STATUS_EVENT = 'agtools:platform-layout-persistence-changed';

export interface UiPreferencesSnapshot {
  values: Record<string, string>;
}

let hydrated = false;
let hydrationPromise: Promise<void> | null = null;
let saving = false;
let pending: string | null = null;
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
      (Array.isArray(parsed.orderedEntryIds) || Array.isArray(parsed.orderedPlatformIds))) {
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

/** Use only after hydration; an existing layout includes explicit empty selections. */
export function hasSavedPlatformLayout(): boolean {
  return layout(readLocal()) != null;
}

function writeLocal(value: string): void {
  try { localStorage.setItem(PLATFORM_LAYOUT_KEY, value); }
  catch (error) { reportError(error); }
}

// At most one save is in flight. A timeout reports failure but does not launch
// another overlapping writer: an IPC timeout cannot cancel the old disk write.
async function flush(): Promise<void> {
  if (!hydrated || saving || pending == null) return;
  saving = true;
  try {
    while (pending != null) {
      const value = pending;
      pending = null;
      const timer = setTimeout(() => reportError(new Error('UI preferences save timeout')), TIMEOUT_MS);
      try {
        await invoke('save_ui_preferences', { values: { [PLATFORM_LAYOUT_KEY]: value } });
        reportError(null);
      } catch (error) {
        pending ??= value;
        reportError(error);
        break;
      } finally { clearTimeout(timer); }
    }
  } finally { saving = false; }
}

export function persistPlatformLayout(value: string): void {
  if (!layout(value)) {
    reportError(new Error('Invalid platform layout'));
    return;
  }
  reportError(null);
  lastRevision = Math.max(lastRevision, revision(readLocal()));
  pending = withRevision(value);
  writeLocal(pending);
  if (hydrated) void flush();
  else void hydrateUiPreferences();
}

export function hydrateUiPreferences(): Promise<void> {
  if (hydrationPromise) return hydrationPromise;
  if (hydrated) return flush();
  const localBefore = readLocal();
  const pendingBefore = pending;
  hydrationPromise = (async () => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const snapshot = await Promise.race([
        invoke<UiPreferencesSnapshot>('load_ui_preferences'),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error('UI preferences load timeout')), TIMEOUT_MS);
        }),
      ]);
      const raw = snapshot?.values?.[PLATFORM_LAYOUT_KEY] ?? null;
      if (raw != null && !layout(raw)) throw new Error('Invalid saved platform layout');
      const local = pending ?? readLocal();
      const usableLocal = layout(local) ? local : null;
      lastRevision = Math.max(lastRevision, revision(raw), revision(usableLocal));
      const edited = pending !== pendingBefore || readLocal() !== localBefore || pendingBefore != null;
      // Legacy conflicts have no timestamp evidence. Preserve the user's
      // existing WebView layout and establish a revision on the first migration.
      const chosen = edited && usableLocal != null ? usableLocal :
        raw != null && (usableLocal == null || revision(raw) > revision(usableLocal)) ? raw : usableLocal;
      if (chosen != null) {
        const next = chosen !== raw && (revision(chosen) === 0 || (edited && revision(chosen) <= revision(raw)))
          ? withRevision(chosen) : chosen;
        writeLocal(next);
        pending = next !== raw ? next : null;
        if (!edited && next !== localBefore && typeof window !== 'undefined') {
          window.dispatchEvent(new Event('agtools:platform-layout-hydrated'));
        }
      }
      hydrated = true;
      reportError(null);
      await flush();
    } catch (error) {
      // Load failure is NOT an empty file. Do not write a cache over unknown
      // durable data; retain pending changes and let an explicit retry reconcile.
      reportError(error);
    } finally {
      if (timer != null) clearTimeout(timer);
      hydrationPromise = null;
    }
  })();
  return hydrationPromise;
}

export async function retryPlatformLayoutPersistence(): Promise<void> {
  if (saving) {
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
