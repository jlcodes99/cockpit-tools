import type { CodexApiKeyBudget, CodexApiKeyBudgetUsage, CodexBudgetPeriod } from '../types/codexRemoteService';

export interface CodexKeyBudgetDraft {
  period: CodexBudgetPeriod;
  amount: string;
  dailyAmount: string;
  maxConcurrent: string;
  dailyImages: string;
}
export function budgetDraft(value?: CodexApiKeyBudget): CodexKeyBudgetDraft {
  return {
    period: value?.period ?? 'lifetime',
    amount: value?.limitMicrousd == null ? '' : String(value.limitMicrousd / 1_000_000),
    dailyAmount: value?.dailyLimitMicrousd == null ? '' : String(value.dailyLimitMicrousd / 1_000_000),
    maxConcurrent: value?.maxConcurrent == null ? '' : String(value.maxConcurrent),
    dailyImages: value?.dailyImageLimit == null ? '' : String(value.dailyImageLimit),
  };
}
export function parseBudgetValue(value: string, money: boolean, max = Number.MAX_SAFE_INTEGER): number | null | undefined {
  if (!value.trim()) return null;
  if (!(money ? /^\d+(?:\.\d{1,6})?$/ : /^\d+$/).test(value.trim())) return undefined;
  const amount = money ? Math.round(Number(value) * 1_000_000) : Number(value);
  return Number.isSafeInteger(amount) && amount > 0 && amount <= max ? amount : undefined;
}
export function parseBudgetDraft(value: CodexKeyBudgetDraft): CodexApiKeyBudget | null {
  const limitMicrousd = parseBudgetValue(value.amount, true, 1_000_000_000_000_000);
  const dailyLimitMicrousd = parseBudgetValue(value.dailyAmount, true, 1_000_000_000_000_000);
  const maxConcurrent = parseBudgetValue(value.maxConcurrent, false, 1024);
  const dailyImageLimit = parseBudgetValue(value.dailyImages, false, 4_294_967_295);
  if (limitMicrousd === undefined || dailyLimitMicrousd === undefined || maxConcurrent === undefined || dailyImageLimit === undefined) return null;
  return { period: value.period, limitMicrousd, dailyLimitMicrousd, maxConcurrent, dailyImageLimit };
}
export function budgetPeriodStart(period: CodexBudgetPeriod, now: number): number {
  const date = new Date(now);
  const midnight = Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate());
  if (period === 'daily') return midnight;
  if (period === 'weekly') return midnight - ((date.getUTCDay() + 6) % 7) * 86_400_000;
  if (period === 'monthly') return Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), 1);
  return 0;
}
export function budgetCurrentUsed(policy: CodexApiKeyBudget, usage?: CodexApiKeyBudgetUsage, now = Date.now()): number {
  if (policy.period === 'lifetime') return usage?.lifetimeMicrousd ?? 0;
  const bucket = usage?.[policy.period];
  return bucket?.start === budgetPeriodStart(policy.period, now) ? bucket.usedMicrousd : 0;
}
export const formatBudgetUsd = (microusd: number) => new Intl.NumberFormat(undefined, {
  style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: microusd > 0 && microusd < 10_000 ? 6 : 2,
}).format(microusd / 1_000_000);
