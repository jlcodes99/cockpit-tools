import { invoke } from '@tauri-apps/api/core';

const PLATFORM_LAYOUT_KEY = 'agtools.platform_layout.v1';

export interface UiPreferencesSnapshot {
  values: Record<string, string>;
}

let hydrated = false;

function readLocal(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeLocal(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // ignore quota / private-mode failures
  }
}

export function persistPlatformLayout(value: string): void {
  writeLocal(PLATFORM_LAYOUT_KEY, value);
  if (!hydrated) return;
  void invoke('save_ui_preferences', {
    values: { [PLATFORM_LAYOUT_KEY]: value },
  }).catch(() => undefined);
}

export async function hydrateUiPreferences(): Promise<void> {
  try {
    const snapshot = await invoke<UiPreferencesSnapshot>('load_ui_preferences');
    const value = snapshot?.values?.[PLATFORM_LAYOUT_KEY];
    if (typeof value === 'string' && readLocal(PLATFORM_LAYOUT_KEY) == null) {
      writeLocal(PLATFORM_LAYOUT_KEY, value);
    }
  } catch {
    // ignore invoke failures
  }
  hydrated = true;
}
