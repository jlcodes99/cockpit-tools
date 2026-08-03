export interface DeepSeekBalanceInfo {
  currency: string;
  totalBalance: string;
  grantedBalance: string;
  toppedUpBalance: string;
}

export interface ModelProviderUsageSummary {
  mode?: string | null;
  isValid?: boolean | null;
  status?: string | null;
  planName?: string | null;
  remaining?: number | null;
  balance?: number | null;
  unit?: string | null;
  isAvailable?: boolean | null;
  balanceInfos?: DeepSeekBalanceInfo[];
  quotaUnlimited?: boolean | null;
  quotaLimit?: number | null;
  quotaUsed?: number | null;
  quotaRemaining?: number | null;
  todayRequests?: number | null;
  todayTotalTokens?: number | null;
  todayCost?: number | null;
  totalRequests?: number | null;
  totalTotalTokens?: number | null;
  totalCost?: number | null;
  modelStatsCount: number;
  latencyMs: number;
  details?: Array<{
    key: string;
    label: string;
    value: string;
  }>;
}

export interface CodexApiKeyUsageCacheEntry {
  loading: boolean;
  summary?: ModelProviderUsageSummary;
  error?: string;
  unavailable?: boolean;
  updatedAt?: number;
}

export function parseCodexApiKeyUsageCache(
  raw: string | null,
): Record<string, CodexApiKeyUsageCacheEntry> {
  if (!raw) return {};
  const parsed = JSON.parse(raw) as Record<string, unknown>;
  if (!parsed || typeof parsed !== 'object') return {};
  return Object.fromEntries(
    Object.entries(parsed)
      .filter(([, value]) => Boolean(value) && typeof value === 'object')
      .map(([accountId, value]) => {
        const entry = value as Omit<CodexApiKeyUsageCacheEntry, 'loading'>;
        return [
          accountId,
          {
            loading: false,
            summary: entry.summary,
            error: typeof entry.error === 'string' ? entry.error : undefined,
            unavailable: entry.unavailable === true,
            updatedAt:
              typeof entry.updatedAt === 'number' && Number.isFinite(entry.updatedAt)
                ? entry.updatedAt
                : undefined,
          },
        ];
      }),
  );
}

export function serializeCodexApiKeyUsageCache(
  value: Record<string, CodexApiKeyUsageCacheEntry>,
): string {
  return JSON.stringify(
    Object.fromEntries(
      Object.entries(value).map(([accountId, entry]) => [
        accountId,
        {
          summary: entry.summary,
          error: entry.error,
          unavailable: entry.unavailable === true,
          updatedAt: entry.updatedAt,
        },
      ]),
    ),
  );
}

export function isOfficialDeepSeekBaseUrl(baseUrl: unknown): boolean {
  try {
    return new URL(String(baseUrl ?? '').trim()).hostname.toLowerCase() === 'api.deepseek.com';
  } catch {
    return false;
  }
}

export function preferredDeepSeekCurrency(language: string): 'CNY' | 'USD' {
  const normalized = language.trim().toLowerCase();
  return normalized === 'zh-cn' || normalized === 'zh-tw' ? 'CNY' : 'USD';
}

export function selectDeepSeekBalanceInfo(
  balanceInfos: DeepSeekBalanceInfo[] | null | undefined,
  language: string,
): DeepSeekBalanceInfo | null {
  if (!balanceInfos?.length) return null;
  const preferred = preferredDeepSeekCurrency(language);
  return (
    balanceInfos.find(
      (balance) => String(balance?.currency ?? '').trim().toUpperCase() === preferred,
    ) ?? balanceInfos[0]
  );
}

export function formatDeepSeekBalanceMoney(
  value: string | null | undefined,
  currency: string | null | undefined,
): string {
  const amount = String(value ?? '').trim();
  if (!amount) return '-';
  const unit = String(currency ?? '').trim().toUpperCase();
  return unit === 'USD' ? `$${amount}` : `${amount}${unit ? ` ${unit}` : ''}`;
}
