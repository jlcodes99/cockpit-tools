import { useEffect, useRef } from 'react';
import { activeGroupTabSettingKey, hasPlatformGroupOnDisk, setActiveGroupTab } from '../services/platformGroupService';
import { hydrateDurableUiSettings, readDurableUiSetting } from '../utils/durableUiSettings';
import { useDurableUiSetting } from './useDurableUiSetting';

export function useActiveGroupTab(
  platform: string,
  loadedGroups?: readonly { id: string }[],
): string | null {
  const key = activeGroupTabSettingKey(platform);
  const selected = useDurableUiSetting(key);
  const groupId = selected && selected !== '__all__' ? selected : null;
  const invalid = loadedGroups && groupId && groupId !== '__ungrouped__'
    && !loadedGroups.some(group => group.id === groupId);
  const lastClear = useRef<{ platform: string; groups: typeof loadedGroups; id: string } | null>(null);

  useEffect(() => {
    if (!invalid || !groupId) return;
    const previous = lastClear.current;
    if (previous?.platform === platform && previous.groups === loadedGroups && previous.id === groupId) return;
    let cancelled = false;
    // An invalid WebView value cannot clear a valid selection loaded from Rust.
    void hydrateDurableUiSettings().then(async () => {
      if (cancelled || readDurableUiSetting(key) !== groupId) return;
      const exists = await hasPlatformGroupOnDisk(platform, groupId);
      if (exists || cancelled || readDurableUiSetting(key) !== groupId) return;
      lastClear.current = { platform, groups: loadedGroups, id: groupId };
      setActiveGroupTab(platform, null);
    }).catch(error => console.warn('[Account group selection]', error));
    return () => { cancelled = true; };
  }, [groupId, invalid, key, loadedGroups, platform]);

  return invalid ? null : groupId;
}
