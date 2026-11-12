import type { Account } from '../types/account';
import type { DisplayGroup, GroupSettings } from '../services/groupService';
import {
  calculateGroupQuota,
  calculateOverallQuota,
} from '../services/groupService';
import { getAntigravityGroupResetTimestamp } from '../presentation/platformAccountPresentation';
import { compareCurrentAccountFirst } from './currentAccountSort';

export type AntigravitySortDirection = 'asc' | 'desc';

export const ANTIGRAVITY_RESET_SORT_PREFIX = 'reset:';
export const DEFAULT_ANTIGRAVITY_SORT_BY = 'overall';
export const DEFAULT_ANTIGRAVITY_SORT_DIRECTION: AntigravitySortDirection = 'desc';

const getAccountQuotas = (account: Account): Record<string, number> => {
  const quotas: Record<string, number> = {};
  if (!account.quota?.models) {
    return quotas;
  }
  for (const model of account.quota.models) {
    quotas[model.name] = model.percentage;
  }
  return quotas;
};

const toDirectionValue = (diff: number, direction: AntigravitySortDirection) =>
  direction === 'desc' ? diff : -diff;

const buildGroupSettings = (displayGroups: DisplayGroup[]): GroupSettings => {
  const settings: GroupSettings = {
    groupMappings: {},
    groupNames: {},
    groupOrder: displayGroups.map((group) => group.id),
    updatedAt: 0,
    updatedBy: 'desktop',
  };

  for (const group of displayGroups) {
    settings.groupNames[group.id] = group.name;
    for (const modelId of group.models) {
      settings.groupMappings[modelId] = group.id;
    }
  }
  return settings;
};

const compareByOverallQuota = (
  a: Account,
  b: Account,
  direction: AntigravitySortDirection,
) => {
  const aQuota = calculateOverallQuota(getAccountQuotas(a));
  const bQuota = calculateOverallQuota(getAccountQuotas(b));
  return toDirectionValue(bQuota - aQuota, direction);
};

const compareByCreatedAt = (
  a: Account,
  b: Account,
  direction: AntigravitySortDirection,
) => toDirectionValue(b.created_at - a.created_at, direction);

const compareByLastSwitchedAt = (
  a: Account,
  b: Account,
  direction: AntigravitySortDirection,
) => {
  const aTime = a.last_switched_at ?? 0;
  const bTime = b.last_switched_at ?? 0;
  if (aTime <= 0) return bTime > 0 ? 1 : 0;
  if (bTime <= 0) return -1;
  return toDirectionValue(bTime - aTime, direction);
};

const compareByGroupReset = (
  a: Account,
  b: Account,
  direction: AntigravitySortDirection,
  group: DisplayGroup,
) => {
  const aReset = getAntigravityGroupResetTimestamp(a, group);
  const bReset = getAntigravityGroupResetTimestamp(b, group);
  if (aReset === null && bReset === null) return 0;
  if (aReset === null) return 1;
  if (bReset === null) return -1;
  return toDirectionValue(bReset - aReset, direction);
};

const compareByGroupQuota = (
  a: Account,
  b: Account,
  sortBy: string,
  direction: AntigravitySortDirection,
  displayGroups: DisplayGroup[],
) => {
  const groupSettings = buildGroupSettings(displayGroups);
  const aGroupQuota = calculateGroupQuota(sortBy, getAccountQuotas(a), groupSettings) ?? 0;
  const bGroupQuota = calculateGroupQuota(sortBy, getAccountQuotas(b), groupSettings) ?? 0;

  if (aGroupQuota !== bGroupQuota) {
    return toDirectionValue(bGroupQuota - aGroupQuota, direction);
  }

  const aOverall = calculateOverallQuota(getAccountQuotas(a));
  const bOverall = calculateOverallQuota(getAccountQuotas(b));
  return toDirectionValue(bOverall - aOverall, direction);
};

export interface AntigravityAccountSortOptions {
  sortBy: string;
  sortDirection: AntigravitySortDirection;
  displayGroups: DisplayGroup[];
  currentAccountId?: string | null;
  customSortOrderIndex?: Map<string, number> | null;
}

export const normalizeAntigravitySortBy = (sortBy: string | null | undefined) => {
  const value = sortBy?.trim();
  return value ? value : DEFAULT_ANTIGRAVITY_SORT_BY;
};

export const normalizeAntigravitySortDirection = (
  sortDirection: string | null | undefined,
): AntigravitySortDirection => (sortDirection === 'asc' ? 'asc' : 'desc');

export const createAntigravityAccountComparator = ({
  sortBy,
  sortDirection,
  displayGroups,
  currentAccountId,
  customSortOrderIndex,
}: AntigravityAccountSortOptions) => {
  const normalizedSortBy = normalizeAntigravitySortBy(sortBy);

  return (a: Account, b: Account) => {
    if (normalizedSortBy === 'custom') {
      const aIndex = customSortOrderIndex?.get(a.id) ?? Number.MAX_SAFE_INTEGER;
      const bIndex = customSortOrderIndex?.get(b.id) ?? Number.MAX_SAFE_INTEGER;
      if (aIndex !== bIndex) {
        return aIndex - bIndex;
      }
      return b.created_at - a.created_at;
    }

    if (normalizedSortBy === 'last_switched_at') {
      return compareByLastSwitchedAt(a, b, sortDirection);
    }

    const currentFirstDiff = compareCurrentAccountFirst(a.id, b.id, currentAccountId);
    if (currentFirstDiff !== 0) {
      return currentFirstDiff;
    }

    if (normalizedSortBy === 'created_at') {
      return compareByCreatedAt(a, b, sortDirection);
    }

    if (normalizedSortBy.startsWith(ANTIGRAVITY_RESET_SORT_PREFIX) && displayGroups.length > 0) {
      const targetGroupId = normalizedSortBy.slice(ANTIGRAVITY_RESET_SORT_PREFIX.length);
      const targetGroup = displayGroups.find((group) => group.id === targetGroupId);
      if (targetGroup) {
        return compareByGroupReset(a, b, sortDirection, targetGroup);
      }
    }

    if (
      normalizedSortBy !== 'default' &&
      normalizedSortBy !== 'overall' &&
      displayGroups.length > 0
    ) {
      return compareByGroupQuota(a, b, normalizedSortBy, sortDirection, displayGroups);
    }

    return compareByOverallQuota(a, b, sortDirection);
  };
};
