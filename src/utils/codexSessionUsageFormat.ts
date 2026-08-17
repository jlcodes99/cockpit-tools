/** Token 紧凑显示，与 CC Switch formatTokensShort 一致：中日文用万/亿，其他语言用 K/M/B。 */

function normalizeLanguageTag(language: string): string {
  return language.toLowerCase().replace(/_/g, "-");
}

function isTraditionalChineseLanguage(normalizedLanguage: string): boolean {
  return (
    normalizedLanguage === "zh-tw" ||
    normalizedLanguage.startsWith("zh-hant") ||
    normalizedLanguage.startsWith("zh-hk") ||
    normalizedLanguage.startsWith("zh-mo")
  );
}

export function formatSessionUsageCount(value: number): string {
  return Math.max(0, value || 0).toLocaleString();
}

export function formatSessionUsageTokensShort(
  value: number,
  lang: string,
  compactDecimals: 1 | 2 = 1,
): string {
  if (!Number.isFinite(value) || value <= 0) return "0";
  const decimals = compactDecimals;
  const normalizedLang = normalizeLanguageTag(lang);
  if (isTraditionalChineseLanguage(normalizedLang)) {
    if (value >= 1e8) return `${(value / 1e8).toFixed(2)} 億`;
    if (value >= 1e4) return `${(value / 1e4).toFixed(decimals)} 萬`;
    return value.toLocaleString("zh-TW");
  }
  if (normalizedLang.startsWith("zh") || normalizedLang.startsWith("ja")) {
    if (value >= 1e8) return `${(value / 1e8).toFixed(2)} 亿`;
    if (value >= 1e4) return `${(value / 1e4).toFixed(decimals)} 万`;
    return value.toLocaleString();
  }
  if (value >= 1e9) return `${(value / 1e9).toFixed(2)}B`;
  if (value >= 1e6) return `${(value / 1e6).toFixed(2)}M`;
  if (value >= 1e3) return `${(value / 1e3).toFixed(decimals)}K`;
  return value.toLocaleString();
}

export function formatSessionUsageCostUsd(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value) || value <= 0) {
    return "$0.0000";
  }
  return `$${value.toFixed(4)}`;
}

export const SESSION_USAGE_SUMMARY_PENDING = "—";

export type SessionUsageSummaryLoadState =
  | "scanning"
  | "updating"
  | "ready"
  | "failed";

export type SessionUsageSummaryStatus = "scanning" | "updating" | null;

export function hasTrustedSessionUsageCache(
  report?: { lastSyncedAt?: number | null } | null,
): boolean {
  return (
    typeof report?.lastSyncedAt === "number" &&
    Number.isFinite(report.lastSyncedAt) &&
    report.lastSyncedAt > 0
  );
}

export function resolveSessionUsageSummaryStatus(
  loadState: SessionUsageSummaryLoadState,
  hasTrustedCache: boolean,
): SessionUsageSummaryStatus {
  if (loadState === "scanning" && !hasTrustedCache) return "scanning";
  if (loadState === "updating" && hasTrustedCache) return "updating";
  return null;
}
