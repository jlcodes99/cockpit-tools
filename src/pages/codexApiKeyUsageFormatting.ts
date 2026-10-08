import { useCallback } from "react";
import type { TFunction } from "i18next";
import {
  formatCockpitApiInteger,
  formatCockpitApiTokenCount,
  getCockpitApiStatsRecord,
  getCockpitApiUsageRecord,
  readCockpitApiOptionalNumber,
  readCockpitApiString,
  toCockpitApiRecord,
  type CockpitApiJsonRecord,
} from "./codexAccountsControllerModel";
import {
  formatModelProviderUsageMoney,
  resolveNewApiQuotaSnapshot,
} from "../services/modelProviderUsageService";
import type { CodexModelProviderUsageSummary } from "../services/codexModelProviderService";
import type { CodexAccount } from "../types/codex";

/**
 * API Key / 模型供应商额度的展示格式化。
 * 从 useCodexAccountsAccessController 拆出，保持原有行为与调用签名不变。
 */
export function useCodexApiKeyUsageFormatting(
  t: TFunction,
  formatDate: (timestamp: number) => string,
) {
  const formatApiKeyUsageMoney = useCallback(
    (value?: number | null, unit?: string | null): string =>
      formatModelProviderUsageMoney(value ?? undefined, unit ?? undefined),
    [],
  );

  const formatApiKeyUsageQuotaValue = useCallback(
    (
      summary: CodexModelProviderUsageSummary | undefined,
      value?: number | null,
    ): string => {
      if (summary?.quotaUnlimited === true) {
        return t("codex.modelProviders.usage.unlimitedQuota", "无限额度");
      }
      return formatApiKeyUsageMoney(value, summary?.unit);
    },
    [formatApiKeyUsageMoney, t],
  );

  const resolveCockpitApiAccountBalanceText = useCallback(
    (account: CodexAccount): string | null => {
      const usage = getCockpitApiUsageRecord(account);
      const stats = getCockpitApiStatsRecord(account);
      const total = toCockpitApiRecord(stats?.total);
      const profile = toCockpitApiRecord(
        toCockpitApiRecord(account.quota?.raw_data)?.profile,
      );
      const records = [usage, total, profile].filter(
        (record): record is CockpitApiJsonRecord => Boolean(record),
      );
      const displayKeys = [
        "balance_display",
        "account_balance_display",
        "wallet_balance_display",
      ];
      for (const record of records) {
        for (const key of displayKeys) {
          const value = readCockpitApiString(record, key);
          if (value) return value;
        }
      }
      const numberKeys = ["balance", "account_balance", "wallet_balance"];
      for (const record of records) {
        for (const key of numberKeys) {
          const value = readCockpitApiOptionalNumber(record, key);
          if (value != null) return formatApiKeyUsageMoney(value, "USD");
        }
      }
      return null;
    },
    [formatApiKeyUsageMoney],
  );

  const formatApiKeyUsagePercent = useCallback(
    (summary?: CodexModelProviderUsageSummary): number => {
      if (summary?.mode === "new_api") {
        const { granted, available } = resolveNewApiQuotaSnapshot(summary);
        if (granted != null && available != null && granted > 0) {
          return Math.max(
            0,
            Math.min(100, Math.round(((granted - available) / granted) * 100)),
          );
        }
      }
      const used = summary?.quotaUsed ?? summary?.totalCost;
      const limit = summary?.quotaLimit;
      if (
        typeof used !== "number" ||
        typeof limit !== "number" ||
        !Number.isFinite(used) ||
        !Number.isFinite(limit) ||
        limit <= 0
      ) {
        return 0;
      }
      return Math.max(0, Math.min(100, Math.round((used / limit) * 100)));
    },
    [],
  );

  const formatApiKeyUsageDetailLabel = useCallback(
    (key: string, fallback: string): string => {
      const labels: Record<string, string> = {
        modelName: t("codex.modelProviders.usage.fields.modelName", "Model"),
        intervalRemaining: t(
          "codex.modelProviders.usage.fields.intervalRemaining",
          "Interval Remaining",
        ),
        intervalLimit: t(
          "codex.modelProviders.usage.fields.intervalLimit",
          "Interval Limit",
        ),
        intervalRemainingPercent: t(
          "codex.modelProviders.usage.fields.intervalRemainingPercent",
          "Interval Remaining %",
        ),
        intervalExpiresAt: t(
          "codex.modelProviders.usage.fields.intervalExpiresAt",
          "Interval Reset",
        ),
        weeklyRemaining: t(
          "codex.modelProviders.usage.fields.weeklyRemaining",
          "Weekly Remaining",
        ),
        weeklyLimit: t(
          "codex.modelProviders.usage.fields.weeklyLimit",
          "Weekly Limit",
        ),
        weeklyRemainingPercent: t(
          "codex.modelProviders.usage.fields.weeklyRemainingPercent",
          "Weekly Remaining %",
        ),
        weeklyExpiresAt: t(
          "codex.modelProviders.usage.fields.weeklyExpiresAt",
          "Weekly Reset",
        ),
        status: t("codex.modelProviders.usage.fields.status", "状态"),
        planName: t("codex.modelProviders.usage.fields.planName", "订阅"),
        remaining: t("codex.modelProviders.usage.fields.remaining", "剩余额度"),
        balance: t("codex.modelProviders.usage.fields.balance", "余额"),
        quotaUnlimited: t(
          "codex.modelProviders.usage.fields.quotaUnlimited",
          "无限额度",
        ),
        todayRequests: t(
          "codex.modelProviders.usage.fields.todayRequests",
          "今日请求",
        ),
        todayTokens: t(
          "codex.modelProviders.usage.fields.todayTokens",
          "今日 Token",
        ),
        todayCost: t("codex.modelProviders.usage.fields.todayCost", "今日消耗"),
        totalRequests: t(
          "codex.modelProviders.usage.fields.totalRequests",
          "累计请求",
        ),
        totalTokens: t(
          "codex.modelProviders.usage.fields.totalTokens",
          "累计 Token",
        ),
        totalCost: t("codex.modelProviders.usage.fields.totalCost", "累计消耗"),
        hardLimitUsd: t(
          "codex.modelProviders.usage.fields.hardLimitUsd",
          "硬额度",
        ),
        softLimitUsd: t(
          "codex.modelProviders.usage.fields.softLimitUsd",
          "软额度",
        ),
        systemHardLimitUsd: t(
          "codex.modelProviders.usage.fields.systemHardLimitUsd",
          "系统额度",
        ),
        accessUntil: t(
          "codex.modelProviders.usage.fields.accessUntil",
          "可用至",
        ),
        expiresAt: t("codex.modelProviders.usage.fields.expiresAt", "过期时间"),
        totalGranted: t(
          "codex.modelProviders.usage.fields.totalGranted",
          "授予额度",
        ),
        totalAvailable: t(
          "codex.modelProviders.usage.fields.totalAvailable",
          "可用额度",
        ),
        modelLimitsEnabled: t(
          "codex.modelProviders.usage.fields.modelLimitsEnabled",
          "模型限制",
        ),
        totalUsage: t(
          "codex.modelProviders.usage.fields.totalUsage",
          "累计消耗",
        ),
        isAvailable: t(
          "codex.modelProviders.usage.fields.isAvailable",
          "余额可用",
        ),
        currency: t("codex.modelProviders.usage.fields.currency", "币种"),
        totalBalance: t(
          "codex.modelProviders.usage.fields.totalBalance",
          "总余额",
        ),
        grantedBalance: t(
          "codex.modelProviders.usage.fields.grantedBalance",
          "赠金余额",
        ),
        toppedUpBalance: t(
          "codex.modelProviders.usage.fields.toppedUpBalance",
          "充值余额",
        ),
      };
      return labels[key] ?? fallback;
    },
    [t],
  );

  const formatApiKeyUsageDetailValue = useCallback(
    (item: { key: string; value: string }, unit?: string | null): string => {
      const raw = item.value.trim();
      const numeric = Number(raw);
      if (
        Number.isFinite(numeric) &&
        (item.key.includes("Tokens") ||
          item.key === "todayTokens" ||
          item.key === "totalTokens")
      ) {
        return formatCockpitApiTokenCount(numeric);
      }
      if (Number.isFinite(numeric) && item.key === "accessUntil") {
        return numeric > 0 ? formatDate(numeric * 1000) : "-";
      }
      if (Number.isFinite(numeric) && item.key === "expiresAt") {
        return numeric > 0 ? formatDate(numeric * 1000) : "-";
      }
      if (
        Number.isFinite(numeric) &&
        (item.key === "intervalExpiresAt" || item.key === "weeklyExpiresAt")
      ) {
        return numeric > 0 ? formatDate(numeric * 1000) : "-";
      }
      if (
        item.key === "quotaUnlimited" ||
        item.key === "modelLimitsEnabled" ||
        item.key === "isAvailable"
      ) {
        if (raw === "true")
          return t("codex.modelProviders.usage.booleanTrue", "是");
        if (raw === "false")
          return t("codex.modelProviders.usage.booleanFalse", "否");
      }
      if (
        Number.isFinite(numeric) &&
        [
          "remaining",
          "balance",
          "todayCost",
          "totalCost",
          "hardLimitUsd",
          "softLimitUsd",
          "systemHardLimitUsd",
          "totalBalance",
          "grantedBalance",
          "toppedUpBalance",
        ].includes(item.key)
      ) {
        return formatApiKeyUsageMoney(numeric, unit);
      }
      if (
        Number.isFinite(numeric) &&
        ["totalGranted", "totalAvailable"].includes(item.key)
      ) {
        return formatCockpitApiInteger(numeric);
      }
      if (Number.isFinite(numeric) && item.key === "totalUsage") {
        return formatApiKeyUsageMoney(numeric / 100, unit);
      }
      if (
        Number.isFinite(numeric) &&
        (item.key.includes("Requests") ||
          item.key === "todayRequests" ||
          item.key === "totalRequests")
      ) {
        return formatCockpitApiInteger(numeric);
      }
      return raw || "-";
    },
    [formatApiKeyUsageMoney, t],
  );

  const findApiKeyUsageDetail = useCallback(
    (summary: CodexModelProviderUsageSummary | undefined, key: string) =>
      summary?.details?.find((item) => item.key === key),
    [],
  );

  const formatApiKeyUsageDetailByKey = useCallback(
    (
      summary: CodexModelProviderUsageSummary | undefined,
      key: string,
    ): string => {
      const detail = findApiKeyUsageDetail(summary, key);
      if (!detail) return "-";
      return formatApiKeyUsageDetailValue(detail, summary?.unit);
    },
    [findApiKeyUsageDetail, formatApiKeyUsageDetailValue],
  );

  return {
    findApiKeyUsageDetail,
    formatApiKeyUsageDetailByKey,
    formatApiKeyUsageDetailLabel,
    formatApiKeyUsageDetailValue,
    formatApiKeyUsageMoney,
    formatApiKeyUsagePercent,
    formatApiKeyUsageQuotaValue,
    resolveCockpitApiAccountBalanceText,
  };
}
