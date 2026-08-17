import type { TFunction } from "i18next";
import type { CodexAccount } from "../types/codex";
import {
  getCodexPlanFilterKey,
  isCodexApiKeyAccount,
  isCodexNewApiAccount,
  isCodexPendingOAuthAccount,
} from "../types/codex";
import type { CodexAccountGroup } from "../services/codexAccountGroupService";
import { splitValidityFilterValues } from "./accountValidityFilter";
import { compareCurrentAccountFirst } from "./currentAccountSort";
import { normalizeAccountsOverviewScope } from "./accountsOverviewFilterPersistence";
import {
  persistUserMemoryList,
  readUserMemoryList,
  USER_MEMORY_LISTS,
} from "./userMemory";

export const CODEX_PRIMARY_PLAN_FILTER_KEYS = [
  "FREE",
  "PLUS",
  "PRO",
  "TEAM",
  "ENTERPRISE",
] as const;
const CODEX_SPECIAL_PLAN_FILTER_KEYS = new Set([
  "PENDING",
  "ERROR",
  "VALID",
  "ZERO_QUOTA",
  "EXPIRED",
]);

/** Special overview filter: accounts with exhausted primary/weekly quota. */
export const CODEX_ZERO_QUOTA_FILTER_VALUE = "ZERO_QUOTA";
/** Special overview filter: subscription expired (or access-token-only expired). */
export const CODEX_EXPIRED_FILTER_VALUE = "EXPIRED";

export const CODEX_OVERVIEW_FILTER_SCOPE =
  normalizeAccountsOverviewScope("Codex");
export const CODEX_OVERVIEW_FILTER_FIELDS = {
  searchQuery: "search_query",
  filterTypes: "filter_types",
  expiryFilter: "expiry_filter",
  tagFilter: "tags",
  groupFilter: "group_filter",
  activeGroupId: "active_group_id",
  sortBy: "sort_by",
  sortDirection: "sort_direction",
} as const;

const CODEX_CUSTOM_SORT_ACTIVE_KEY =
  "agtools.codex.accounts.custom_sort_active.v1";

export type CodexOverviewSortDirection = "asc" | "desc";

export interface CodexPlanFilterCounts {
  all: number;
  VALID: number;
  ERROR: number;
  ZERO_QUOTA: number;
  EXPIRED: number;
  counts: Record<string, number>;
}

export interface CodexOverviewFilterOption {
  value: string;
  label: string;
}

export function normalizeCodexPlanFilterValue(value: string): string {
  return value.trim().toUpperCase();
}

export function sortCodexPlanFilterKeys(values: string[]): string[] {
  const uniqueKeys = Array.from(
    new Set(values.map(normalizeCodexPlanFilterValue).filter(Boolean)),
  );
  const primaryOrder = new Map<string, number>(
    CODEX_PRIMARY_PLAN_FILTER_KEYS.map((key, index) => [key, index]),
  );
  return uniqueKeys.sort((left, right) => {
    const leftOrder = primaryOrder.get(left);
    const rightOrder = primaryOrder.get(right);
    if (leftOrder !== undefined || rightOrder !== undefined) {
      return (leftOrder ?? Number.MAX_SAFE_INTEGER) -
        (rightOrder ?? Number.MAX_SAFE_INTEGER);
    }
    return left.localeCompare(right);
  });
}

export function createCodexPlanFilterCounts(
  total: number,
): CodexPlanFilterCounts {
  return {
    all: total,
    VALID: 0,
    ERROR: 0,
    ZERO_QUOTA: 0,
    EXPIRED: 0,
    counts: {},
  };
}

/** OAuth-style quota exhausted: known weekly/hourly remaining percent is 0. */
export function isCodexOverviewAccountZeroQuota(account: CodexAccount): boolean {
  if (isCodexPendingOAuthAccount(account) || isCodexNewApiAccount(account)) {
    return false;
  }
  if (isCodexApiKeyAccount(account)) {
    return false;
  }
  const hourly = account.quota?.hourly_percentage;
  const weekly = account.quota?.weekly_percentage;
  const hasHourly = typeof hourly === "number" && Number.isFinite(hourly);
  const hasWeekly = typeof weekly === "number" && Number.isFinite(weekly);
  if (!hasHourly && !hasWeekly) return false;
  // Treat as zero-quota when every reported window is fully used.
  if (hasHourly && hourly! > 0) return false;
  if (hasWeekly && weekly! > 0) return false;
  return true;
}

export function isCodexOverviewAccountSubscriptionExpired(
  account: CodexAccount,
): boolean {
  if (isCodexPendingOAuthAccount(account) || isCodexNewApiAccount(account)) {
    return false;
  }
  if (isCodexApiKeyAccount(account)) return false;
  const until = account.subscription_active_until?.trim();
  if (!until) return false;
  const parsed = Date.parse(until);
  if (Number.isNaN(parsed)) return false;
  return parsed <= Date.now();
}

export function incrementCodexPlanFilterCount(
  counts: CodexPlanFilterCounts,
  value: string,
): void {
  const key = normalizeCodexPlanFilterValue(value);
  if (!key) return;
  counts.counts[key] = (counts.counts[key] ?? 0) + 1;
}

function getCodexPlanFilterCount(
  counts: CodexPlanFilterCounts,
  value: string,
): number {
  return counts.counts[normalizeCodexPlanFilterValue(value)] ?? 0;
}

export function buildCodexPlanFilterOptions(
  counts: CodexPlanFilterCounts,
  options?: {
    includeValid?: boolean;
    includePending?: boolean;
    includeError?: boolean;
    includeZeroQuota?: boolean;
    includeExpired?: boolean;
    pendingLabel?: string;
    errorLabel?: string;
    zeroQuotaLabel?: string;
    expiredLabel?: string;
    validOption?: CodexOverviewFilterOption;
  },
): CodexOverviewFilterOption[] {
  const usedKeys = new Set<string>();
  const result: CodexOverviewFilterOption[] = [];

  for (const key of CODEX_PRIMARY_PLAN_FILTER_KEYS) {
    usedKeys.add(key);
    result.push({
      value: key,
      label: `${key} (${getCodexPlanFilterCount(counts, key)})`,
    });
  }

  Object.keys(counts.counts)
    .map(normalizeCodexPlanFilterValue)
    .filter(
      (key) =>
        key &&
        !usedKeys.has(key) &&
        !CODEX_SPECIAL_PLAN_FILTER_KEYS.has(key),
    )
    .sort((left, right) => left.localeCompare(right))
    .forEach((key) => {
      usedKeys.add(key);
      result.push({
        value: key,
        label: `${key} (${getCodexPlanFilterCount(counts, key)})`,
      });
    });

  if (options?.includePending ?? true) {
    result.push({
      value: "PENDING",
      label: `${options?.pendingLabel ?? "待授权"} (${getCodexPlanFilterCount(
        counts,
        "PENDING",
      )})`,
    });
  }
  if (options?.includeError ?? true) {
    result.push({
      value: "ERROR",
      label: `${options?.errorLabel ?? "ERROR"} (${counts.ERROR})`,
    });
  }
  if (options?.includeZeroQuota) {
    result.push({
      value: CODEX_ZERO_QUOTA_FILTER_VALUE,
      label: `${options.zeroQuotaLabel ?? "0% 额度"} (${counts.ZERO_QUOTA})`,
    });
  }
  if (options?.includeExpired) {
    result.push({
      value: CODEX_EXPIRED_FILTER_VALUE,
      label: `${options.expiredLabel ?? "已过期"} (${counts.EXPIRED})`,
    });
  }
  if (options?.includeValid && options.validOption) {
    result.push(options.validOption);
  }

  return result;
}

export function buildCodexOverviewSortOptions(
  t: TFunction,
): CodexOverviewFilterOption[] {
  return [
    {
      value: "created_at",
      label: t("common.shared.sort.createdAt", "按创建时间"),
    },
    {
      value: "weekly",
      label: t("codex.sort.weekly", "按周配额"),
    },
    {
      value: "hourly",
      label: t("codex.sort.hourly", "按5小时配额"),
    },
    {
      value: "weekly_reset",
      label: t("codex.sort.weeklyReset", "按周配额重置时间"),
    },
    {
      value: "hourly_reset",
      label: t("codex.sort.hourlyReset", "按5小时配额重置时间"),
    },
    {
      value: "subscription_expiry",
      label: t("codex.sort.subscriptionExpiry", "按订阅有效期"),
    },
    {
      value: "custom",
      label: t("codex.sort.custom", "自定义顺序"),
    },
  ];
}

export function buildCodexOverviewGroupFilterOptions(
  groups: CodexAccountGroup[],
): CodexOverviewFilterOption[] {
  return groups
    .map((group) => ({
      value: group.id,
      label: `${group.name} (${group.accountIds.length})`,
    }))
    .sort((left, right) => left.label.localeCompare(right.label));
}

export function collectCodexOverviewAvailableTags(
  accounts: CodexAccount[],
): string[] {
  const tags = new Set<string>();
  accounts.forEach((account) => {
    (account.tags || []).forEach((tag) => {
      const normalized = tag.trim().toLowerCase();
      if (normalized) tags.add(normalized);
    });
  });
  return Array.from(tags).sort((left, right) => left.localeCompare(right));
}

export function isCodexOverviewAccountAbnormal(
  account: CodexAccount,
): boolean {
  if (isCodexPendingOAuthAccount(account)) return false;
  if (account.requires_reauth === true) return true;

  const rawMessage = account.quota_error?.message?.trim() ?? "";
  if (!rawMessage) return false;
  const lowerRawMessage = rawMessage.toLowerCase();
  const statusCode =
    rawMessage.match(/API 返回错误\s+(\d{3})/i)?.[1] ||
    rawMessage.match(/status[=: ]+(\d{3})/i)?.[1] ||
    "";
  const errorCode = (
    account.quota_error?.code ||
    rawMessage.match(/\[error_code:([^\]]+)\]/)?.[1] ||
    rawMessage.match(/error_code[=:]\s*([^,\]\s]+)/i)?.[1] ||
    ""
  )
    .trim()
    .toLowerCase();

  if (
    errorCode === "unsupported_country_region_territory" ||
    lowerRawMessage.includes("unsupported_country_region_territory") ||
    lowerRawMessage.includes("当前网络地区不支持刷新 codex 授权")
  ) {
    return false;
  }

  return (
    statusCode === "401" ||
    errorCode === "deactivated_workspace" ||
    errorCode === "refresh_token_reused" ||
    errorCode === "refresh_token_expired" ||
    errorCode === "refresh_token_invalidated" ||
    errorCode === "token_invalidated" ||
    errorCode === "invalid_grant" ||
    errorCode === "invalid_token" ||
    lowerRawMessage.includes("refresh_token_reused") ||
    lowerRawMessage.includes("deactivated_workspace") ||
    lowerRawMessage.includes("refresh_token_expired") ||
    lowerRawMessage.includes("refresh_token_invalidated") ||
    lowerRawMessage.includes("token_invalidated") ||
    lowerRawMessage.includes("refresh_token 已被其它客户端或实例使用过") ||
    lowerRawMessage.includes("your authentication token has been invalidated") ||
    lowerRawMessage.includes("401 unauthorized") ||
    lowerRawMessage.includes("invalid_grant") ||
    lowerRawMessage.includes("token 已过期且无 refresh_token") ||
    lowerRawMessage.includes("缺少 refresh_token") ||
    lowerRawMessage.includes("token 已过期且刷新失败") ||
    lowerRawMessage.includes("刷新 token 失败")
  );
}

interface CodexOverviewComparatorOptions {
  sortBy: string;
  sortDirection: CodexOverviewSortDirection;
  customSortOrder: string[];
  currentAccountId: string | null;
  resolveSubscriptionTimestamp: (account: CodexAccount) => number | null;
}

export function createCodexOverviewAccountComparator({
  sortBy,
  sortDirection,
  customSortOrder,
  currentAccountId,
  resolveSubscriptionTimestamp,
}: CodexOverviewComparatorOptions): (
  left: CodexAccount,
  right: CodexAccount,
) => number {
  const customSortOrderIndex = new Map<string, number>();
  customSortOrder.forEach((accountId, index) => {
    customSortOrderIndex.set(accountId, index);
  });

  return (left, right) => {
    if (sortBy === "custom") {
      const leftIndex =
        customSortOrderIndex.get(left.id) ?? Number.MAX_SAFE_INTEGER;
      const rightIndex =
        customSortOrderIndex.get(right.id) ?? Number.MAX_SAFE_INTEGER;
      if (leftIndex !== rightIndex) {
        return leftIndex - rightIndex;
      }
      return right.created_at - left.created_at;
    }

    const cockpitApiPriority =
      Number(!isCodexNewApiAccount(left)) -
      Number(!isCodexNewApiAccount(right));
    if (cockpitApiPriority !== 0) {
      return cockpitApiPriority;
    }

    const currentFirstDiff = compareCurrentAccountFirst(
      left.id,
      right.id,
      currentAccountId,
    );
    if (currentFirstDiff !== 0) {
      return currentFirstDiff;
    }

    if (sortBy === "created_at") {
      const diff = right.created_at - left.created_at;
      return sortDirection === "desc" ? diff : -diff;
    }
    if (sortBy === "weekly_reset" || sortBy === "hourly_reset") {
      const leftReset =
        sortBy === "weekly_reset"
          ? (left.quota?.weekly_reset_time ?? null)
          : (left.quota?.hourly_reset_time ?? null);
      const rightReset =
        sortBy === "weekly_reset"
          ? (right.quota?.weekly_reset_time ?? null)
          : (right.quota?.hourly_reset_time ?? null);
      if (leftReset == null && rightReset == null) return 0;
      if (leftReset == null) return 1;
      if (rightReset == null) return -1;
      return sortDirection === "desc"
        ? rightReset - leftReset
        : leftReset - rightReset;
    }
    if (sortBy === "subscription_expiry") {
      const leftExpiry = resolveSubscriptionTimestamp(left);
      const rightExpiry = resolveSubscriptionTimestamp(right);
      if (leftExpiry == null && rightExpiry == null) return 0;
      if (leftExpiry == null) return 1;
      if (rightExpiry == null) return -1;
      return sortDirection === "desc"
        ? rightExpiry - leftExpiry
        : leftExpiry - rightExpiry;
    }

    const leftValue =
      sortBy === "weekly"
        ? (left.quota?.weekly_percentage ?? -1)
        : (left.quota?.hourly_percentage ?? -1);
    const rightValue =
      sortBy === "weekly"
        ? (right.quota?.weekly_percentage ?? -1)
        : (right.quota?.hourly_percentage ?? -1);
    return sortDirection === "desc"
      ? rightValue - leftValue
      : leftValue - rightValue;
  };
}

interface FilterCodexOverviewAccountsOptions {
  accounts: CodexAccount[];
  groups: CodexAccountGroup[];
  searchQuery: string;
  filterTypes: string[];
  tagFilter: string[];
  groupFilter: string[];
  activeGroupId: string | null;
  resolveDisplayName: (account: CodexAccount) => string;
  compareAccounts: (left: CodexAccount, right: CodexAccount) => number;
  isAbnormalAccount?: (account: CodexAccount) => boolean;
}

export function filterAndSortCodexOverviewAccounts({
  accounts,
  groups,
  searchQuery,
  filterTypes,
  tagFilter,
  groupFilter,
  activeGroupId,
  resolveDisplayName,
  compareAccounts,
  isAbnormalAccount = isCodexOverviewAccountAbnormal,
}: FilterCodexOverviewAccountsOptions): CodexAccount[] {
  let result = [...accounts];
  if (searchQuery.trim()) {
    const query = searchQuery.toLowerCase();
    result = result.filter((account) =>
      resolveDisplayName(account).toLowerCase().includes(query),
    );
  }
  if (filterTypes.length > 0) {
    const { requireValidAccounts, selectedTypes } =
      splitValidityFilterValues(filterTypes);
    if (requireValidAccounts) {
      result = result.filter((account) => !isAbnormalAccount(account));
    }
    if (selectedTypes.size > 0) {
      result = result.filter((account) => {
        const matchesSpecial =
          (selectedTypes.has("ERROR") && isAbnormalAccount(account)) ||
          (selectedTypes.has(CODEX_ZERO_QUOTA_FILTER_VALUE) &&
            isCodexOverviewAccountZeroQuota(account)) ||
          (selectedTypes.has(CODEX_EXPIRED_FILTER_VALUE) &&
            isCodexOverviewAccountSubscriptionExpired(account));
        const planKeys = Array.from(selectedTypes).filter(
          (key) =>
            key !== "ERROR" &&
            key !== CODEX_ZERO_QUOTA_FILTER_VALUE &&
            key !== CODEX_EXPIRED_FILTER_VALUE,
        );
        if (planKeys.length === 0) {
          return matchesSpecial;
        }
        const matchesPlan = selectedTypes.has(getCodexPlanFilterKey(account));
        // Multi-select is OR across plan tiers and special status filters.
        return matchesPlan || matchesSpecial;
      });
    }
  }
  if (tagFilter.length > 0) {
    const selectedTags = new Set(
      tagFilter.map((tag) => tag.trim().toLowerCase()),
    );
    result = result.filter((account) =>
      (account.tags || [])
        .map((tag) => tag.trim().toLowerCase())
        .some((tag) => selectedTags.has(tag)),
    );
  }
  if (groupFilter.length > 0) {
    const existingGroupIds = new Set(groups.map((group) => group.id));
    const activeFilter = groupFilter.filter((id) => existingGroupIds.has(id));
    if (activeFilter.length > 0) {
      const groupAccountIds = new Set<string>();
      const selectedGroupIds = new Set(activeFilter);
      for (const group of groups) {
        if (selectedGroupIds.has(group.id)) {
          for (const accountId of group.accountIds) {
            groupAccountIds.add(accountId);
          }
        }
      }
      result = result.filter((account) => groupAccountIds.has(account.id));
    }
  }
  if (activeGroupId) {
    const scopedGroup = groups.find((group) => group.id === activeGroupId);
    if (!scopedGroup) {
      return [];
    }
    const scopedIds = new Set(scopedGroup.accountIds);
    result = result.filter((account) => scopedIds.has(account.id));
  }
  result.sort(compareAccounts);
  return result;
}

export function readCodexCustomSortOrder(): string[] {
  return readUserMemoryList(USER_MEMORY_LISTS.codexCustomSort);
}

export function writeCodexCustomSortOrder(accountIds: string[]): void {
  persistUserMemoryList(USER_MEMORY_LISTS.codexCustomSort, accountIds);
}

export function readCodexCustomSortActive(): boolean {
  try {
    return localStorage.getItem(CODEX_CUSTOM_SORT_ACTIVE_KEY) === "1";
  } catch {
    return false;
  }
}

export function writeCodexCustomSortActive(active: boolean): void {
  try {
    localStorage.setItem(CODEX_CUSTOM_SORT_ACTIVE_KEY, active ? "1" : "0");
  } catch {
    // ignore persistence failures
  }
}
