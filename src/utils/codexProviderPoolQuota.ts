import { isCodexApiKeyAccount, isCodexChatgptPlanAccount, type CodexAccount } from '../types/codex';
import { resolveCodexAccountDisplayName } from './codexAccountDisplayName';

export interface CodexProviderPoolQuota {
  id: string;
  name: string;
  baseUrl?: string;
  mode?: string;
  remaining: number | null;
  total: number | null;
  unit: string | null;
  unlimited: boolean;
  status: 'ready' | 'stale' | 'failed' | 'unavailable';
  observedAt: number | null;
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value != null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function finiteNumber(value: unknown): number | null {
  if (typeof value !== 'number' && (typeof value !== 'string' || !value.trim())) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function readProviderQuota(account: CodexAccount) {
  const raw = record(account.quota?.raw_data);
  const summary = record(raw?.provider_usage);
  if (summary) {
    const quotaRemaining = finiteNumber(summary.quotaRemaining);
    return {
      remaining: quotaRemaining ?? finiteNumber(summary.balance) ?? finiteNumber(summary.remaining),
      total: quotaRemaining === null ? null : finiteNumber(summary.quotaLimit),
      unit: text(summary.unit),
      unlimited: summary.quotaUnlimited === true,
      mode: text(summary.mode) ?? undefined,
      invalid: summary.isValid === false || raw?.provider_usage_query_failed === true,
    };
  }

  if (raw?.object === 'codex_cockpit_api_quota') {
    const profile = record(raw.profile);
    const usage = record(raw.usage) ?? record(profile?.usage);
    const display = record(usage?.display) ?? record(profile?.display);
    const displayUnit = text(display?.unit);
    const unlimited = usage?.unlimited_quota === true;
    // Raw counters have no verified unit. In particular, the legacy
    // hourly_percentage is a ratio, never a provider currency or a 5h window.
    return {
      remaining: displayUnit ? finiteNumber(display?.remaining) : unlimited ? null : finiteNumber(usage?.total_available),
      total: displayUnit ? finiteNumber(display?.total) : unlimited ? null : finiteNumber(usage?.total_granted),
      unit: displayUnit,
      unlimited,
      mode: 'new_api',
      invalid: raw.provider_usage_query_failed === true,
    };
  }

  return { remaining: null, total: null, unit: null, unlimited: false, mode: undefined, invalid: raw?.provider_usage_query_failed === true };
}

export function summarizeCodexProviderPoolQuotas(
  accounts: readonly CodexAccount[],
  now: number,
): CodexProviderPoolQuota[] {
  return accounts
    .filter((account) => isCodexApiKeyAccount(account) && !isCodexChatgptPlanAccount(account))
    .map((account) => {
      const { invalid, ...quota } = readProviderQuota(account);
      const timestamp = account.usage_updated_at;
      const observedAt = typeof timestamp === 'number' && Number.isFinite(timestamp) && timestamp > 0 ? timestamp : null;
      const hasValue = quota.remaining != null || quota.unlimited;
      const status = account.quota_error || invalid ? 'failed'
        : !hasValue ? 'unavailable'
        : observedAt === null || observedAt > now || now - observedAt > 900 ? 'stale'
        : 'ready';
      return {
        ...quota,
        id: account.id,
        name: account.api_provider_name?.trim() || resolveCodexAccountDisplayName(account),
        baseUrl: account.api_base_url ?? undefined,
        status,
        observedAt,
      };
    });
}
