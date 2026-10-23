import type { CodexAccount } from '../types/codex';
import type { CodexAccountGroup } from '../services/codexAccountGroupService';
import type { CodexLocalAccessAccountHealth } from '../types/codexLocalAccess';
import {
  codexServiceDisplayRows, codexServiceRelatedEndpointIds, getCodexServiceEndpoint,
  getCodexServicePlanFilterKey, getCodexServiceSourceId,
} from './chatgptPlanOverview';
import {
  createCodexPlanFilterCounts, filterAndSortCodexOverviewAccounts, incrementCodexPlanFilterCount,
  isCodexOverviewAccountAbnormal, isCodexOverviewAccountSubscriptionExpired, isCodexOverviewAccountZeroQuota,
} from './codexAccountOverview';

/** Filter identity is the source; credentials and live health stay per endpoint. */
export function buildCodexServiceMemberView(input: {
  accounts: readonly CodexAccount[];
  sourceAccounts: readonly CodexAccount[];
  groups: readonly CodexAccountGroup[];
  memberIds: readonly string[];
  accountHealth?: readonly CodexLocalAccessAccountHealth[];
  officialAccountBindings?: Readonly<Record<string, string>>;
}) {
  const rows = codexServiceDisplayRows(input.accounts);
  const endpoints = new Map(input.accounts.map((account) => [account.id, account]));
  const sources = new Map(input.sourceAccounts.map((account) => [account.id, account]));
  const members = new Set(input.memberIds);
  const health = new Map(input.accountHealth?.map((item) => [item.accountId, item]));
  const selectedBySource = new Map<string, string[]>();
  for (const account of input.accounts) {
    if (!members.has(account.id)) continue;
    const id = getCodexServiceSourceId(account);
    const selected = selectedBySource.get(id) ?? [];
    selected.push(account.id);
    selectedBySource.set(id, selected);
  }
  const source = (account: CodexAccount) => sources.get(getCodexServiceSourceId(account)) ?? getCodexServiceEndpoint(account);
  const isAbnormal = (account: CodexAccount) => {
    const selected = selectedBySource.get(getCodexServiceSourceId(account)) ?? [];
    return (selected.length ? selected : [account.id]).every((id) => {
      const endpoint = endpoints.get(id);
      if (!endpoint) return true;
      if (!input.officialAccountBindings?.[id] && isCodexOverviewAccountAbnormal(getCodexServiceEndpoint(endpoint))) return true;
      const state = health.get(id);
      return Boolean(state && (!state.available || state.schedulerAvailable === false || state.cooldowns.length));
    });
  };
  const isZeroQuota = (account: CodexAccount) => isCodexOverviewAccountZeroQuota(source(account));
  const isExpired = (account: CodexAccount) => isCodexOverviewAccountSubscriptionExpired(source(account));
  const counts = createCodexPlanFilterCounts(rows.length);
  for (const row of rows) {
    incrementCodexPlanFilterCount(counts, getCodexServicePlanFilterKey(row));
    counts[isAbnormal(row) ? 'ERROR' : 'VALID'] += 1;
    if (isZeroQuota(row)) counts.ZERO_QUOTA += 1;
    if (isExpired(row)) counts.EXPIRED += 1;
  }
  const groups = input.groups.map((group) => ({ ...group, accountIds: [...new Set(group.accountIds.map((id) => {
    const endpoint = endpoints.get(id);
    return endpoint ? getCodexServiceSourceId(endpoint) : id;
  }))] }));
  return { rows, counts, groups, source, isAbnormal, isZeroQuota, isExpired };
}

export function filterCodexServiceMemberView(
  view: ReturnType<typeof buildCodexServiceMemberView>,
  options: {
    searchQuery: string;
    filterTypes: string[];
    tagFilter: string[];
    groupFilter: string[];
    activeGroupId?: string | null;
    resolveDisplayName: (account: CodexAccount) => string;
    compareAccounts: (left: CodexAccount, right: CodexAccount) => number;
  },
): CodexAccount[] {
  return filterAndSortCodexOverviewAccounts({
    ...options, accounts: view.rows, groups: view.groups, activeGroupId: options.activeGroupId ?? null,
    resolveAccountId: getCodexServiceSourceId, resolvePlanKey: getCodexServicePlanFilterKey,
    isAbnormalAccount: view.isAbnormal, isZeroQuotaAccount: view.isZeroQuota, isSubscriptionExpiredAccount: view.isExpired,
    resolveDisplayName: (account) => [options.resolveDisplayName(account),
      ...view.groups.filter((group) => group.accountIds.includes(getCodexServiceSourceId(account))).map((group) => group.name)].join(' '),
    compareAccounts: (left, right) => options.compareAccounts(view.source(left), view.source(right)),
  });
}

/** Count visible identities without rewriting the persisted scope IDs. */
export function countCodexServiceSourceAccounts(ids: readonly string[], accounts: readonly CodexAccount[]): number {
  const byId = new Map(accounts.map((account) => [account.id, account]));
  return new Set(ids.map((id) => {
    const account = byId.get(id);
    return account ? getCodexServiceSourceId(account) : id;
  })).size;
}

/** Checkbox, batch selection and priority changes share the same add gate. */
export function needsCodexServiceMemberAuthorizationChoice(
  account: CodexAccount,
  accounts: readonly CodexAccount[],
  selected: ReadonlySet<string>,
  persistedIds: readonly string[],
  shouldRecommend: (account: CodexAccount) => boolean,
): boolean {
  const related = codexServiceRelatedEndpointIds(account, accounts);
  return !related.some((id) => selected.has(id) || persistedIds.includes(id)) && shouldRecommend(account);
}
