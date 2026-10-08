import {
  getPlatformLayoutPersistenceError,
  hasSavedPlatformLayout,
  hydrateUiPreferences,
} from './uiPreferences';

/** A lost WebView migration marker must never turn saved layout choices into defaults. */
export async function initializeClassicSidebar({ isCurrent, initialize, markInitialized }: {
  isCurrent(): boolean;
  initialize(): void;
  markInitialized(): void;
}): Promise<void> {
  await hydrateUiPreferences();
  if (!isCurrent() || getPlatformLayoutPersistenceError() != null) return;
  if (!hasSavedPlatformLayout()) initialize();
  markInitialized();
}
