export const PLATFORM_LAYOUT_ICON_STORAGE_KEY =
  "agtools.platform_layout.custom_icons.v1";

interface StorageWriter {
  setItem(key: string, value: string): void;
}

export function persistPlatformLayoutCustomIcons(
  storage: StorageWriter,
  customIcons: unknown,
): boolean {
  try {
    storage.setItem(
      PLATFORM_LAYOUT_ICON_STORAGE_KEY,
      JSON.stringify(customIcons),
    );
    return true;
  } catch {
    return false;
  }
}
