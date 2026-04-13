/**
 * CodeBuddy Suite 页面共享 Hook
 *
 * 封装 CodeBuddy CN 和 WorkBuddy 页面的共享逻辑
 */

import { useMemo, useCallback } from 'react';
import type { CodebuddySuiteAccountBase } from '../types/codebuddy-suite';
import { KNOWN_PLAN_FILTERS } from '../components/codebuddy-suite/CodebuddySuiteConfig';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import { splitValidityFilterValues } from '../utils/accountValidityFilter';

const QUOTA_NUMBER_FORMATTER = new Intl.NumberFormat('en-US', {
  maximumFractionDigits: 2,
});

export function formatQuotaNumber(value: number): string {
  if (!Number.isFinite(value)) return '0';
  return QUOTA_NUMBER_FORMATTER.format(Math.max(0, value));
}

export function clampPercent(value: number | null | undefined): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

export function getQuotaClassByRemainPercent(remainPercent: number | null): string {
  if (remainPercent == null || !Number.isFinite(remainPercent)) return 'high';
  if (remainPercent <= 10) return 'critical';
  if (remainPercent <= 30) return 'low';
  if (remainPercent <= 60) return 'medium';
  return 'high';
}

export interface UseCodebuddySuitePageOptions<TAccount extends CodebuddySuiteAccountBase> {
  accounts: TAccount[];
  currentAccountId?: string | null;
  searchQuery: string;
  filterTypes: string[];
  tagFilter: string[];
  sortDirection: 'asc' | 'desc';
  getPlanBadge: (account: TAccount) => string;
  isAbnormalAccount: (account: TAccount) => boolean;
  normalizeTag: (tag: string) => string;
  groupByTag: boolean;
}

export interface UseCodebuddySuitePageReturn<TAccount extends CodebuddySuiteAccountBase> {
  tierSummary: {
    all: number;
    validCount: number;
    dynamicCounts: Map<string, number>;
    extraKeys: string[];
  };
  filteredAccounts: TAccount[];
  filteredIds: string[];
  groupedAccounts: Array<[string, TAccount[]]>;
  resolvePlanKey: (account: TAccount) => string;
  resolveTierBadgeClass: (plan: string) => string;
  formatQuotaDateTime: (timeMs: number | null) => string;
}

export function useCodebuddySuitePage<TAccount extends CodebuddySuiteAccountBase>(
  options: UseCodebuddySuitePageOptions<TAccount>
): UseCodebuddySuitePageReturn<TAccount> {
  const {
    accounts,
    currentAccountId,
    searchQuery,
    filterTypes,
    tagFilter,
    sortDirection,
    getPlanBadge,
    isAbnormalAccount,
    normalizeTag,
    groupByTag,
  } = options;

  const untaggedKey = '__untagged__';

  const resolvePlanKey = useCallback(
    (account: TAccount) => getPlanBadge(account),
    [getPlanBadge]
  );

  const resolveTierBadgeClass = useCallback((plan: string) => {
    switch (plan.toUpperCase()) {
      case 'FREE':
        return 'free';
      case 'TRIAL':
        return 'trial';
      case 'PRO':
        return 'pro';
      case 'ENTERPRISE':
        return 'enterprise';
      default:
        return 'unknown';
    }
  }, []);

  const tierSummary = useMemo(() => {
    const dynamicCounts = new Map<string, number>();
    accounts.forEach((account) => {
      const tier = resolvePlanKey(account);
      dynamicCounts.set(tier, (dynamicCounts.get(tier) ?? 0) + 1);
    });
    const extraKeys = Array.from(dynamicCounts.keys())
      .filter((tier) => !(KNOWN_PLAN_FILTERS as readonly string[]).includes(tier))
      .sort((a, b) => a.localeCompare(b));
    const validCount = accounts.reduce(
      (count, account) => (isAbnormalAccount(account) ? count : count + 1),
      0,
    );
    return { all: accounts.length, validCount, dynamicCounts, extraKeys };
  }, [accounts, isAbnormalAccount, resolvePlanKey]);

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter((account) =>
        [account.email, account.nickname || '', account.uid || '', account.enterprise_name || '', account.id]
          .some((item) => item.toLowerCase().includes(query))
      );
    }
    if (filterTypes.length > 0) {
      const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(filterTypes);
      if (requireValidAccounts) {
        result = result.filter((account) => !isAbnormalAccount(account));
      }
      if (selectedTypes.size > 0) {
        result = result.filter((account) => selectedTypes.has(resolvePlanKey(account)));
      }
    }
    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      result = result.filter((acc) =>
        (acc.tags || []).map(normalizeTag).some((tag) => selectedTags.has(tag))
      );
    }
    result.sort((a, b) => {
      const currentFirstDiff = compareCurrentAccountFirst(a.id, b.id, currentAccountId);
      if (currentFirstDiff !== 0) {
        return currentFirstDiff;
      }

      const diff = b.created_at - a.created_at;
      return sortDirection === 'desc' ? diff : -diff;
    });
    return result;
  }, [accounts, currentAccountId, searchQuery, filterTypes, isAbnormalAccount, resolvePlanKey, tagFilter, normalizeTag, sortDirection]);

  const filteredIds = useMemo(
    () => filteredAccounts.map((account) => account.id),
    [filteredAccounts]
  );

  const groupedAccounts = useMemo(() => {
    if (!groupByTag) return [] as Array<[string, TAccount[]]>;
    const groups = new Map<string, TAccount[]>();
    const selectedTags = new Set(tagFilter.map(normalizeTag));
    filteredAccounts.forEach((account) => {
      const tags = (account.tags || []).map(normalizeTag).filter(Boolean);
      const matchedTags = selectedTags.size > 0 ? tags.filter((tag) => selectedTags.has(tag)) : tags;
      if (matchedTags.length === 0) {
        if (!groups.has(untaggedKey)) groups.set(untaggedKey, []);
        groups.get(untaggedKey)?.push(account);
        return;
      }
      matchedTags.forEach((tag) => {
        if (!groups.has(tag)) groups.set(tag, []);
        groups.get(tag)?.push(account);
      });
    });
    return Array.from(groups.entries()).sort(([aKey], [bKey]) => {
      if (aKey === untaggedKey) return 1;
      if (bKey === untaggedKey) return -1;
      return aKey.localeCompare(bKey);
    });
  }, [filteredAccounts, groupByTag, normalizeTag, tagFilter, untaggedKey]);

  const formatQuotaDateTime = useCallback((timeMs: number | null): string => {
    if (timeMs == null || !Number.isFinite(timeMs)) return '';
    const date = new Date(timeMs);
    const year = date.getFullYear();
    const month = String(date.getMonth() + 1).padStart(2, '0');
    const day = String(date.getDate()).padStart(2, '0');
    const hour = String(date.getHours()).padStart(2, '0');
    const minute = String(date.getMinutes()).padStart(2, '0');
    const second = String(date.getSeconds()).padStart(2, '0');
    return `${year}年 ${month}月${day}日 ${hour}:${minute}:${second}`;
  }, []);

  return {
    tierSummary,
    filteredAccounts,
    filteredIds,
    groupedAccounts,
    resolvePlanKey,
    resolveTierBadgeClass,
    formatQuotaDateTime,
  };
}
