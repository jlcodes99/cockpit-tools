import type { CodexAccount } from '../types/codex';
import {
  getCodexEffectiveQuotaPercentages,
  getCodexPlanFilterKey,
} from '../types/codex';

export const CODEX_QUOTA_POOL_PLAN_KEYS = [
  'FREE',
  'PLUS',
  'PRO',
  'TEAM',
  'ENTERPRISE',
] as const;

export type CodexQuotaPoolPlanKey = (typeof CODEX_QUOTA_POOL_PLAN_KEYS)[number];

export interface CodexQuotaPoolItem {
  key: CodexQuotaPoolPlanKey | 'ALL';
  count: number;
  hourly: number;
  weekly: number;
}

export interface CodexQuotaPoolSummary {
  all: CodexQuotaPoolItem;
  byPlan: Record<CodexQuotaPoolPlanKey, CodexQuotaPoolItem>;
  visiblePlans: CodexQuotaPoolItem[];
}

function createQuotaPoolItem(key: CodexQuotaPoolItem['key']): CodexQuotaPoolItem {
  return { key, count: 0, hourly: 0, weekly: 0 };
}

function addAccountToQuotaPool(target: CodexQuotaPoolItem, account: CodexAccount): void {
  const percentages = getCodexEffectiveQuotaPercentages(account.quota);
  target.count += 1;
  target.hourly += percentages.hourly ?? 0;
  target.weekly += percentages.weekly ?? 0;
}

function isQuotaPoolPlanKey(value: string): value is CodexQuotaPoolPlanKey {
  return CODEX_QUOTA_POOL_PLAN_KEYS.includes(value as CodexQuotaPoolPlanKey);
}

export function summarizeCodexQuotaPool(accounts: CodexAccount[]): CodexQuotaPoolSummary {
  const byPlan = CODEX_QUOTA_POOL_PLAN_KEYS.reduce(
    (next, key) => {
      next[key] = createQuotaPoolItem(key);
      return next;
    },
    {} as Record<CodexQuotaPoolPlanKey, CodexQuotaPoolItem>,
  );
  const all = createQuotaPoolItem('ALL');

  accounts.forEach((account) => {
    addAccountToQuotaPool(all, account);
    const planKey = getCodexPlanFilterKey(account);
    if (isQuotaPoolPlanKey(planKey)) {
      addAccountToQuotaPool(byPlan[planKey], account);
    }
  });

  return {
    all,
    byPlan,
    visiblePlans: CODEX_QUOTA_POOL_PLAN_KEYS.map((key) => byPlan[key]).filter(
      (item) => item.count > 0,
    ),
  };
}

export function formatCodexQuotaPoolPercent(value: number): string {
  return `${Math.max(0, Math.round(value))}%`;
}
