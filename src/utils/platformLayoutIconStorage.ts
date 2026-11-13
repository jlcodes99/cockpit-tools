import { readDurableUiSetting, saveDurableUiSetting } from './durableUiSettings';

export const PLATFORM_LAYOUT_ICON_STORAGE_KEY = 'agtools.platform_layout.custom_icons.v1';
export const PLATFORM_LAYOUT_ICONS_CHANGED_EVENT = 'agtools:platform-layout-icons-changed';

export function readPlatformLayoutCustomIcons(): string | null {
  return readDurableUiSetting(PLATFORM_LAYOUT_ICON_STORAGE_KEY);
}

export async function persistPlatformLayoutCustomIcons(customIcons: unknown): Promise<boolean> {
  try {
    await saveDurableUiSetting(PLATFORM_LAYOUT_ICON_STORAGE_KEY, JSON.stringify(customIcons), true);
    return true;
  } catch (error) {
    console.warn('[Platform layout] Could not save custom icons:', error);
    return false;
  }
}
