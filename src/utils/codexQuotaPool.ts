import type { CodexAccount } from '../types/codex';
import {
  getCodexPlanFilterKey,
  getCodexPlanDisplayName,
  isCodexChatgptPlanAccount,
  getCodexQuotaWindows,
} from '../types/codex';
import { sortCodexPlanFilterKeys } from './codexAccountOverview';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';
import { chatGptPlanBindingStatus, chatGptPlanOverviewId, getCodexServiceAccountDisplay, getCodexServiceEndpoint, getCodexServicePlanFilterKey, getCodexServiceSourceId, resolveCodexServiceDisplaySource, type ChatGptPlanBindingStatus } from './chatgptPlanOverview';

export interface CodexQuotaPoolWindow {
  key: string;
  label: string;
  percentage: number;
  accountCount: number;
  windowMinutes: number;
}

export interface CodexQuotaPoolItem {
  key: string;
  label: string;
  official: boolean;
  bindingStatus?: ChatGptPlanBindingStatus;
  unknownPlan?: boolean;
  count: number;
  windows: CodexQuotaPoolWindow[];
}

export interface CodexQuotaPoolSummary {
  all: CodexQuotaPoolItem;
  byPlan: Record<string, CodexQuotaPoolItem>;
  visiblePlans: CodexQuotaPoolItem[];
}

function createQuotaPoolItem(key: string, label = key, official = false): CodexQuotaPoolItem {
  return { key, label, official, count: 0, windows: [] };
}

function addAccountToQuotaPool(target: CodexQuotaPoolItem, account: CodexAccount, includeQuota = !isCodexChatgptPlanAccount(account)): void {
  target.count += 1;
  if (!includeQuota) return;
  const seenWindowKeys = new Set<string>();
  getCodexQuotaWindows(account.quota).forEach((window) => {
    const key = window.label.trim().toLowerCase();
    const windowMinutes =
      window.windowMinutes ?? (window.id === 'secondary' ? 7 * 24 * 60 : 5 * 60);
    let pooledWindow = target.windows.find((item) => item.key === key);
    if (!pooledWindow) {
      pooledWindow = {
        key,
        label: window.label,
        percentage: 0,
        accountCount: 0,
        windowMinutes,
      };
      target.windows.push(pooledWindow);
    }
    pooledWindow.percentage += window.percentage;
    pooledWindow.windowMinutes = Math.min(pooledWindow.windowMinutes, windowMinutes);
    if (!seenWindowKeys.has(key)) {
      pooledWindow.accountCount += 1;
      seenWindowKeys.add(key);
    }
  });
  target.windows.sort(
    (left, right) =>
      left.windowMinutes - right.windowMinutes || left.label.localeCompare(right.label),
  );
}

export function summarizeCodexQuotaPool(accounts: CodexAccount[], context?: {
  sourceAccounts: readonly CodexAccount[];
  grants: readonly ChatGptPlanAccount[];
  displaySourceIds?: ReadonlyMap<string, string>;
}): CodexQuotaPoolSummary {
  const byPlan: Record<string, CodexQuotaPoolItem> = {};
  const all = createQuotaPoolItem('ALL');
  const grants = context?.grants ?? [];
  const seenSources = new Set<string>();
  accounts.forEach((row) => {
    const account = getCodexServiceEndpoint(row);
    const official = isCodexChatgptPlanAccount(account);
    const grant = official ? grants.find((item) => chatGptPlanOverviewId(item.id) === account.id) : undefined;
    const bindingStatus = grant ? chatGptPlanBindingStatus(grant) : undefined;
    const source = context ? resolveCodexServiceDisplaySource(account, context.sourceAccounts, grants, context.displaySourceIds) : undefined;
    // Quota is the original account's observed allowance, not a fabricated
    // allowance for a second credential. Count one source even with two routes.
    const sourceId = source?.id ?? (context ? account.id : getCodexServiceSourceId(row));
    if (seenSources.has(sourceId)) return;
    seenSources.add(sourceId);
    const linkedDisplay = !context && sourceId !== account.id;
    const display = source ?? (linkedDisplay ? getCodexServiceAccountDisplay(row) : account);
    const rawPlan = bindingStatus === 'invalid' || (bindingStatus === 'pending' && !source)
      ? undefined : display.plan_type?.trim() || display.auth_file_plan_type?.trim();
    const label = source ? getCodexPlanFilterKey(source)
      : linkedDisplay ? getCodexServicePlanFilterKey(row)
      : official ? rawPlan ? getCodexPlanDisplayName(rawPlan) : 'ChatGPT'
      : getCodexPlanFilterKey(account);
    const planKey = label === 'ChatGPT' ? bindingStatus ? `CHATGPT_PLAN:${bindingStatus.toUpperCase()}` : 'CHATGPT_PLAN' : label;
    // A bound official route shares the normal subscription category.
    byPlan[planKey] ??= createQuotaPoolItem(planKey, label);
    if (bindingStatus && !source && !linkedDisplay) byPlan[planKey].bindingStatus = bindingStatus;
    if (official && !rawPlan && !source && !linkedDisplay) byPlan[planKey].unknownPlan = true;
    // For a projected endpoint, consume only its explicit source quota. The
    // endpoint itself remains untouched for routing and health checks.
    const includeQuota = Boolean(source) || linkedDisplay || !official;
    addAccountToQuotaPool(all, display, includeQuota);
    addAccountToQuotaPool(byPlan[planKey], display, includeQuota);
  });
  const planOrder = new Map(sortCodexPlanFilterKeys(Object.values(byPlan).map((item) => item.label)).map((key, index) => [key, index]));

  return {
    all,
    byPlan,
    visiblePlans: Object.values(byPlan).sort((a, b) =>
      planOrder.get(a.label.toUpperCase())! - planOrder.get(b.label.toUpperCase())!
      || Number(a.official) - Number(b.official) || a.key.localeCompare(b.key)),
  };
}

export function formatCodexQuotaPoolPlanLabel(item: CodexQuotaPoolItem, _officialBadge: string, statusLabels?: { pending: string; invalid: string }): string {
  const label = item.unknownPlan && (item.bindingStatus === 'pending' || item.bindingStatus === 'invalid')
    ? statusLabels?.[item.bindingStatus] ?? item.label : item.label;
  return label;
}

export function formatCodexQuotaPoolPercent(value: number): string {
  return `${Math.max(0, Math.round(value))}%`;
}

export function formatCodexQuotaPoolWindowLabel(
  label: string,
  weeklyLabel: string,
): string {
  return label === 'Weekly' ? weeklyLabel : label;
}
