import type {
  CodebuddySuiteAccountBase,
  CodebuddyUsage,
  OfficialQuotaResource,
  QuotaCategoryGroup,
} from './codebuddy-suite';

export interface ZcodeAccount extends CodebuddySuiteAccountBase {
  auth_mode?: 'oauth' | 'api_key';
  provider: 'zai' | 'bigmodel' | string;
  user_id?: string | null;
  display_name?: string | null;
  avatar_url?: string | null;
  zcode_jwt_token?: string;
  api_key?: string | null;
  team_plan?: { organization_id: string; project_id: string } | null;
  quota_total?: number | null;
  quota_used?: number | null;
  quota_remaining?: number | null;
  quota_reset_at?: number | null;
  usage_updated_at?: number | null;
  user_info_raw?: unknown;
  subscription_raw?: unknown;
  quota_raw?: unknown;
}

type UnknownRecord = Record<string, unknown>;

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function finite(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function remainClass(remainPercent: number | null): string {
  if (remainPercent == null) return 'high';
  if (remainPercent <= 10) return 'critical';
  if (remainPercent <= 30) return 'low';
  if (remainPercent <= 60) return 'medium';
  return 'high';
}

function balanceResources(account: ZcodeAccount): OfficialQuotaResource[] {
  if (!Array.isArray(account.quota_raw)) return [];
  return account.quota_raw.filter(isRecord).map((item, index) => {
    const total = Math.max(0, finite(item.total_units) ?? 0);
    const used = Math.max(0, finite(item.used_units) ?? 0);
    const remain = Math.max(0, finite(item.remaining_units) ?? finite(item.available_units) ?? total - used);
    const usedPercent = total > 0 ? Math.max(0, Math.min(100, (used / total) * 100)) : 0;
    const periodEnd = finite(item.period_end) ?? finite(item.expires_at);
    const reset = finite(item.next_refresh_at) ?? periodEnd;
    return {
      packageCode: text(item.entitlement_id) ?? `zcode-${index}`,
      packageName: text(item.show_name) ?? text(item.plan_id) ?? `ZCode ${index + 1}`,
      cycleStartTime: null,
      cycleEndTime: null,
      deductionEndTime: null,
      expiredTime: null,
      total,
      remain,
      used,
      usedPercent,
      remainPercent: total > 0 ? (remain / total) * 100 : null,
      refreshAt: reset ? reset * 1000 : null,
      expireAt: periodEnd ? periodEnd * 1000 : null,
      isBasePackage: true,
    };
  });
}

export function getZcodeAccountDisplayEmail(account: ZcodeAccount): string {
  const email = account.email?.trim();
  if (email && email !== 'unknown@zcode.local') return email;
  return account.display_name || account.user_id || email || account.id;
}

export function getZcodePlanBadge(account: ZcodeAccount): string {
  if (account.auth_mode === 'api_key') return 'API Key';
  if (account.team_plan) {
    const level = account.plan_type?.trim();
    return level ? `Team · ${level}` : 'Team';
  }
  return account.plan_type?.trim() || account.provider || 'UNKNOWN';
}

export function getZcodeUsage(account: ZcodeAccount): CodebuddyUsage {
  const total = finite(account.quota_total);
  const used = finite(account.quota_used);
  const percent = total != null && total > 0 && used != null ? (used / total) * 100 : null;
  return {
    isNormal: !account.quota_query_last_error,
    inlineSuggestionsUsedPercent: percent,
    chatMessagesUsedPercent: percent,
    allowanceResetAt: account.quota_reset_at ? account.quota_reset_at * 1000 : null,
  };
}

export function getZcodeQuotaGroups(
  account: ZcodeAccount,
  t: (key: string, defaultValue?: string) => string,
): QuotaCategoryGroup[] {
  const items = balanceResources(account);
  const total = items.reduce((sum, item) => sum + item.total, 0);
  const used = items.reduce((sum, item) => sum + item.used, 0);
  const remain = items.reduce((sum, item) => sum + item.remain, 0);
  const usedPercent = total > 0 ? (used / total) * 100 : 0;
  const remainPercent = total > 0 ? (remain / total) * 100 : null;
  return [
    {
      key: 'base',
      label: t('zcode.quota.models', '模型额度'),
      used,
      total,
      remain,
      usedPercent,
      remainPercent,
      quotaClass: remainClass(remainPercent),
      items,
      visible: items.length > 0,
    },
  ];
}

export function hasZcodeQuotaData(account: ZcodeAccount): boolean {
  return Array.isArray(account.quota_raw) && account.quota_raw.length > 0;
}
