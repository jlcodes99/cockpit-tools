import { useCallback, useState, useSyncExternalStore, type SetStateAction } from 'react';
import {
  areDurableUiSettingsHydrated,
  previewDurableUiSetting,
  readDurableUiSetting,
  subscribeDurableUiSetting,
  type DurableUiSettingKey,
} from '../utils/durableUiSettings';

const noop = () => {};

export function useDurableUiSetting(key: DurableUiSettingKey | undefined): string | null {
  const subscribe = useCallback((listener: () => void) => key ? subscribeDurableUiSetting(key, listener) : noop, [key]);
  const getSnapshot = () => key ? readDurableUiSetting(key) : null;
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export function useDurableUiBooleanSetting(key: DurableUiSettingKey | undefined) {
  const stored = useDurableUiSetting(key);
  const [temporary, setTemporary] = useState(false);
  const setValue = useCallback((next: SetStateAction<boolean>) => {
    if (!key) {
      setTemporary(next);
      return;
    }
    const current = readDurableUiSetting(key) === '1';
    const value = typeof next === 'function' ? next(current) : next;
    if (value !== current || !areDurableUiSettingsHydrated()) {
      previewDurableUiSetting(key, value ? '1' : '0');
    }
  }, [key]);
  return [key ? stored === '1' : temporary, setValue] as const;
}
