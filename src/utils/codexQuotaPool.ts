import type { CodexAccount } from "../types/codex";
import {
  getCodexPlanFilterKey,
  getCodexQuotaWindows,
  isCodexApiKeyAccount,
} from "../types/codex";
import { sortCodexPlanFilterKeys } from "./codexAccountOverview";

export interface CodexQuotaPoolWindow {
  key: string;
  label: string;
  percentage: number;
  accountCount: number;
  windowMinutes: number;
}

export interface CodexQuotaPoolItem {
  key: string;
  count: number;
  balance?: number;
  windows: CodexQuotaPoolWindow[];
}

export interface CodexQuotaPoolSummary {
  all: CodexQuotaPoolItem;
  byPlan: Record<string, CodexQuotaPoolItem>;
  visiblePlans: CodexQuotaPoolItem[];
}

function createQuotaPoolItem(key: CodexQuotaPoolItem['key']): CodexQuotaPoolItem {
  return { key, count: 0, windows: [] };
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value !== "string" || !value.trim()) return undefined;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function getCodexApiKeyBalance(account: CodexAccount): number | undefined {
  if (!isCodexApiKeyAccount(account)) return undefined;
  const raw = asRecord(account.quota?.raw_data);
  if (!raw) return undefined;
  const providerUsage = asRecord(raw.provider_usage);
  if (providerUsage && providerUsage.unit !== "%") {
    const totalAvailable = Array.isArray(providerUsage.details)
      ? providerUsage.details
          .map(asRecord)
          .find((detail) => detail?.key === "totalAvailable")
      : null;
    const providerBalance =
      providerUsage.mode === "new_api"
        ? (asNumber(totalAvailable?.value) ??
          asNumber(providerUsage.quotaRemaining) ??
          asNumber(providerUsage.remaining) ??
          asNumber(providerUsage.balance))
        : (asNumber(providerUsage.remaining) ??
          asNumber(providerUsage.balance) ??
          asNumber(providerUsage.quotaRemaining) ??
          asNumber(totalAvailable?.value));
    if (providerBalance !== undefined) return providerBalance;
  }
  const profile = asRecord(raw.profile);
  const usage = asRecord(raw.usage) ?? asRecord(profile?.usage);
  return (
    asNumber(usage?.total_available) ??
    asNumber(raw.total_available) ??
    asNumber(usage?.balance) ??
    asNumber(raw.balance)
  );
}

function addAccountToQuotaPool(
  target: CodexQuotaPoolItem,
  account: CodexAccount,
): void {
  target.count += 1;
  const balance = getCodexApiKeyBalance(account);
  if (balance !== undefined) {
    target.balance = (target.balance ?? 0) + balance;
  }
  if (isCodexApiKeyAccount(account)) return;

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

export function summarizeCodexQuotaPool(accounts: CodexAccount[]): CodexQuotaPoolSummary {
  const byPlan: Record<string, CodexQuotaPoolItem> = {};
  const all = createQuotaPoolItem('ALL');

  accounts.forEach((account) => {
    addAccountToQuotaPool(all, account);
    const planKey = getCodexPlanFilterKey(account);
    byPlan[planKey] ??= createQuotaPoolItem(planKey);
    addAccountToQuotaPool(byPlan[planKey], account);
  });

  return {
    all,
    byPlan,
    visiblePlans: sortCodexPlanFilterKeys(Object.keys(byPlan)).map(
      (key) => byPlan[key],
    ),
  };
}

export function formatCodexQuotaPoolPercent(value: number): string {
  return `${Math.max(0, Math.round(value))}%`;
}

export function formatCodexQuotaPoolBalance(value: number): string {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(
    Math.max(0, value),
  );
}

export function formatCodexQuotaPoolWindowLabel(
  label: string,
  weeklyLabel: string,
): string {
  return label === 'Weekly' ? weeklyLabel : label;
}
