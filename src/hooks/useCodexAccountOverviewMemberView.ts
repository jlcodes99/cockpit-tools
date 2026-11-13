import { useActiveGroupTab } from "./useActiveGroupTab";
import { useDurableUiSetting } from "./useDurableUiSetting";
import { useAccountsOverviewFilterState } from "./useAccountsOverviewFilterState";
import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import { useTranslation } from "react-i18next";
import type { CodexLocalAccessMemberViewConfig } from "../components/CodexLocalAccessModal";
import type { CodexAccountGroup } from "../services/codexAccountGroupService";
import {
  getCodexPlanFilterKey,
  getCodexSubscriptionPresentationForAccount,
  isCodexApiKeyAccount,
  type CodexAccount,
} from "../types/codex";
import { buildValidAccountsFilterOption } from "../utils/accountValidityFilter";
import {
  readAccountsOverviewFilterField,
  readAccountsOverviewFilterPersistenceEnabled,
  readAccountsOverviewFilterStringArray,
  resolveAccountSortMode,
} from "../utils/accountsOverviewFilterPersistence";
import {
  CODEX_OVERVIEW_FILTER_FIELDS,
  CODEX_OVERVIEW_FILTER_SCOPE,
  buildCodexOverviewGroupFilterOptions,
  buildCodexPlanFilterOptions,
  collectCodexOverviewAvailableTags,
  createCodexOverviewAccountComparator,
  createCodexPlanFilterCounts,
  filterAndSortCodexOverviewAccounts,
  incrementCodexPlanFilterCount,
  isCodexOverviewAccountAbnormal,
  isCodexOverviewAccountSubscriptionExpired,
  isCodexOverviewAccountZeroQuota,
  CODEX_CUSTOM_SORT_ACTIVE_KEY,
  readCodexCustomSortOrder,
  type CodexOverviewSortDirection,
} from "../utils/codexAccountOverview";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import { subscribeUserMemory } from "../utils/userMemory";

interface UseCodexAccountOverviewMemberViewOptions {
  accounts: CodexAccount[];
  groups: CodexAccountGroup[];
  currentAccountId: string | null;
}

function readPersistedString(field: string, fallback: string): string {
  if (!readAccountsOverviewFilterPersistenceEnabled(CODEX_OVERVIEW_FILTER_SCOPE)) {
    return fallback;
  }
  const value = readAccountsOverviewFilterField<unknown>(
    CODEX_OVERVIEW_FILTER_SCOPE,
    field,
    fallback,
  );
  return typeof value === "string" ? value : fallback;
}

function readPersistedStringArray(field: string): string[] {
  if (!readAccountsOverviewFilterPersistenceEnabled(CODEX_OVERVIEW_FILTER_SCOPE)) {
    return [];
  }
  return readAccountsOverviewFilterStringArray(
    CODEX_OVERVIEW_FILTER_SCOPE,
    field,
  );
}

function readPersistedSortDirection(): CodexOverviewSortDirection {
  return readPersistedString(
    CODEX_OVERVIEW_FILTER_FIELDS.sortDirection,
    "desc",
  ) === "asc"
    ? "asc"
    : "desc";
}

export function useCodexAccountOverviewMemberView({
  accounts,
  groups,
  currentAccountId,
}: UseCodexAccountOverviewMemberViewOptions): CodexLocalAccessMemberViewConfig {
  const { t } = useTranslation();
  const [searchQuery, setSearchQuery] = useAccountsOverviewFilterState(
    CODEX_OVERVIEW_FILTER_SCOPE, CODEX_OVERVIEW_FILTER_FIELDS.searchQuery,
    () => readPersistedString(CODEX_OVERVIEW_FILTER_FIELDS.searchQuery, ""),
  );
  const [filterTypes, setFilterTypes] = useAccountsOverviewFilterState(
    CODEX_OVERVIEW_FILTER_SCOPE, CODEX_OVERVIEW_FILTER_FIELDS.filterTypes,
    () => readPersistedStringArray(CODEX_OVERVIEW_FILTER_FIELDS.filterTypes),
  );
  const [tagFilter, setTagFilter] = useAccountsOverviewFilterState(
    CODEX_OVERVIEW_FILTER_SCOPE, CODEX_OVERVIEW_FILTER_FIELDS.tagFilter,
    () => readPersistedStringArray(CODEX_OVERVIEW_FILTER_FIELDS.tagFilter),
  );
  const [groupFilter, setGroupFilter] = useAccountsOverviewFilterState(
    CODEX_OVERVIEW_FILTER_SCOPE, CODEX_OVERVIEW_FILTER_FIELDS.groupFilter,
    () => readPersistedStringArray(CODEX_OVERVIEW_FILTER_FIELDS.groupFilter),
  );
  const [rememberedSortBy] = useAccountsOverviewFilterState(
    CODEX_OVERVIEW_FILTER_SCOPE, CODEX_OVERVIEW_FILTER_FIELDS.sortBy,
    () => readPersistedString(CODEX_OVERVIEW_FILTER_FIELDS.sortBy, "created_at"),
  );
  const sortBy = resolveAccountSortMode(
    rememberedSortBy, useDurableUiSetting(CODEX_CUSTOM_SORT_ACTIVE_KEY), "created_at",
  );
  const [sortDirection] = useAccountsOverviewFilterState(
    CODEX_OVERVIEW_FILTER_SCOPE, CODEX_OVERVIEW_FILTER_FIELDS.sortDirection, readPersistedSortDirection,
  );
  const selectedGroupId = useActiveGroupTab("codex");
  const activeGroupId = useDurableUiSetting("agtools.codex.accounts_overview_filters.persist_enabled") === "0"
    ? null : selectedGroupId;

  const accountPresentations = useMemo(() => {
    const result = new Map<
      string,
      ReturnType<typeof buildCodexAccountPresentation>
    >();
    accounts.forEach((account) => {
      result.set(account.id, buildCodexAccountPresentation(account, t));
    });
    return result;
  }, [accounts, t]);

  const resolveDisplayName = useCallback(
    (account: CodexAccount) =>
      accountPresentations.get(account.id)?.displayName ??
      buildCodexAccountPresentation(account, t).displayName,
    [accountPresentations, t],
  );

  const [customSortEpoch, setCustomSortEpoch] = useState(0);
  useEffect(() => subscribeUserMemory(() => setCustomSortEpoch((value) => value + 1)), []);
  const customSortOrder = useMemo(readCodexCustomSortOrder, [accounts, customSortEpoch]);
  const compareAccounts = useMemo(
    () =>
      createCodexOverviewAccountComparator({
        sortBy,
        sortDirection,
        customSortOrder,
        currentAccountId,
        resolveSubscriptionTimestamp: (account) =>
          isCodexApiKeyAccount(account)
            ? null
            : getCodexSubscriptionPresentationForAccount(account, t)
                .timestampMs,
      }),
    [customSortOrder, currentAccountId, sortBy, sortDirection, t],
  );

  const filteredAccounts = useMemo(
    () =>
      filterAndSortCodexOverviewAccounts({
        accounts,
        groups,
        searchQuery,
        filterTypes,
        tagFilter,
        groupFilter,
        activeGroupId,
        resolveDisplayName,
        compareAccounts,
      }),
    [
      accounts,
      activeGroupId,
      compareAccounts,
      filterTypes,
      groupFilter,
      groups,
      resolveDisplayName,
      searchQuery,
      tagFilter,
    ],
  );

  const tierCounts = useMemo(() => {
    const counts = createCodexPlanFilterCounts(accounts.length);
    accounts.forEach((account) => {
      if (!isCodexOverviewAccountAbnormal(account)) {
        counts.VALID += 1;
      }
      incrementCodexPlanFilterCount(
        counts,
        getCodexPlanFilterKey(account),
      );
      if (isCodexOverviewAccountAbnormal(account)) {
        counts.ERROR += 1;
      }
      if (isCodexOverviewAccountZeroQuota(account)) {
        counts.ZERO_QUOTA += 1;
      }
      if (isCodexOverviewAccountSubscriptionExpired(account)) {
        counts.EXPIRED += 1;
      }
    });
    return counts;
  }, [accounts]);

  const tierFilterOptions = useMemo(
    () =>
      buildCodexPlanFilterOptions(tierCounts, {
        includeValid: true,
        includeZeroQuota: true,
        includeExpired: true,
        pendingLabel: t("codex.pendingAuth.badge", "待授权"),
        chatgptPlanLabel: t("chatgptPlan.title"),
        errorLabel: t("codex.filters.authError", "授权失败"),
        zeroQuotaLabel: t("codex.filters.zeroQuota", "0% 额度"),
        expiredLabel: t("codex.filters.expired", "已过期"),
        validOption: buildValidAccountsFilterOption(t, tierCounts.VALID),
      }),
    [t, tierCounts],
  );
  const availableTags = useMemo(
    () => collectCodexOverviewAvailableTags(accounts),
    [accounts],
  );
  const groupFilterOptions = useMemo(
    () => buildCodexOverviewGroupFilterOptions(groups),
    [groups],
  );

  const toggleArrayValue = useCallback(
    (
      setter: Dispatch<SetStateAction<string[]>>,
      value: string,
    ) => {
      setter((current) =>
        current.includes(value)
          ? current.filter((item) => item !== value)
          : [...current, value],
      );
    },
    [],
  );

  return {
    activeGroupId,
    compareAccounts,
    accounts: filteredAccounts,
    searchQuery,
    filterTypes,
    tagFilter,
    groupFilter,
    tierFilterOptions,
    tierFilterAllLabel: t("common.shared.filter.all", {
      count: tierCounts.all,
    }),
    availableTags,
    groupFilterOptions,
    onSearchQueryChange: setSearchQuery,
    onToggleFilterType: (value) => toggleArrayValue(setFilterTypes, value),
    onClearFilterTypes: () => setFilterTypes([]),
    onToggleTagFilter: (value) => toggleArrayValue(setTagFilter, value),
    onClearTagFilter: () => setTagFilter([]),
    onToggleGroupFilter: (value) => toggleArrayValue(setGroupFilter, value),
    onClearGroupFilter: () => setGroupFilter([]),
  };
}
