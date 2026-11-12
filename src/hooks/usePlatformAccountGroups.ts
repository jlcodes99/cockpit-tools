import { useState, useEffect, useCallback, useMemo } from 'react';
import {
  AccountGroup,
  getPlatformGroups,
  removeAccountsFromPlatformGroup,
  setActiveGroupTab,
} from '../services/platformGroupService';
import { invalidateCache as invalidateLegacyCache } from '../services/accountGroupService';
import { useActiveGroupTab } from './useActiveGroupTab';

const EMPTY_GROUPS: AccountGroup[] = [];

export interface UsePlatformAccountGroupsReturn {
  platform: string;
  groups: AccountGroup[];
  activeGroupId: string | null;
  activeGroup: AccountGroup | null;
  setActiveGroupId: (id: string | null) => void;
  reloadGroups: () => Promise<void>;
  showManageModal: boolean;
  setShowManageModal: (show: boolean) => void;
  showAddToGroupModal: boolean;
  setShowAddToGroupModal: (show: boolean) => void;
  filterAccountsByGroup: <T extends { id: string }>(rawAccounts: T[]) => T[];
  handleRemoveFromGroup: (accountIds: string[]) => Promise<void>;
}

export function usePlatformAccountGroups(
  platform: string,
  onClearSelection?: () => void
): UsePlatformAccountGroupsReturn {
  const [loadedGroups, setLoadedGroups] = useState<Record<string, AccountGroup[]>>({});
  const platformGroups = loadedGroups[platform];
  const groups = platformGroups ?? EMPTY_GROUPS;
  const activeGroupId = useActiveGroupTab(platform, platformGroups);
  const [showManageModal, setShowManageModal] = useState(false);
  const [showAddToGroupModal, setShowAddToGroupModal] = useState(false);

  const reloadGroups = useCallback(async () => {
    try {
      const list = await getPlatformGroups(platform);
      setLoadedGroups(previous => ({ ...previous, [platform]: list }));
    } catch (err) {
      console.error(`[usePlatformAccountGroups] Failed to load groups for ${platform}:`, err);
    }
  }, [platform]);

  useEffect(() => {
    reloadGroups();
  }, [reloadGroups]);

  const setActiveGroupId = useCallback(
    (id: string | null) => {
      setActiveGroupTab(platform, id);
      onClearSelection?.();
    },
    [platform, onClearSelection]
  );

  const activeGroup = useMemo(
    () => (activeGroupId && activeGroupId !== '__ungrouped__' ? groups.find((g) => g.id === activeGroupId) ?? null : null),
    [groups, activeGroupId]
  );

  const filterAccountsByGroup = useCallback(
    <T extends { id: string }>(rawAccounts: T[]): T[] => {
      if (!activeGroupId) return rawAccounts;
      if (activeGroupId === '__ungrouped__') {
        const allGroupedIds = new Set<string>();
        for (const group of groups) {
          for (const id of group.accountIds) {
            allGroupedIds.add(id);
          }
        }
        return rawAccounts.filter((acc) => !allGroupedIds.has(acc.id));
      }
      const group = groups.find((g) => g.id === activeGroupId);
      if (!group) return rawAccounts;
      const groupAccountSet = new Set(group.accountIds);
      return rawAccounts.filter((acc) => groupAccountSet.has(acc.id));
    },
    [activeGroupId, groups]
  );

  const handleRemoveFromGroup = useCallback(
    async (accountIds: string[]) => {
      if (!activeGroupId || accountIds.length === 0) return;
      try {
        await removeAccountsFromPlatformGroup(platform, activeGroupId, accountIds);
        if (platform === 'antigravity') {
          invalidateLegacyCache();
        }
        await reloadGroups();
        onClearSelection?.();
      } catch (err) {
        console.error(`[usePlatformAccountGroups] Failed to remove accounts from group:`, err);
      }
    },
    [activeGroupId, platform, reloadGroups, onClearSelection]
  );

  return {
    platform,
    groups,
    activeGroupId,
    activeGroup,
    setActiveGroupId,
    reloadGroups,
    showManageModal,
    setShowManageModal,
    showAddToGroupModal,
    setShowAddToGroupModal,
    filterAccountsByGroup,
    handleRemoveFromGroup,
  };
}
