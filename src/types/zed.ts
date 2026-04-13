export interface ZedAccount {
  id: string;
  user_id: string;
  github_login: string;
  display_name?: string | null;
  avatar_url?: string | null;
  plan_raw?: string | null;
  subscription_status?: string | null;
  has_overdue_invoices?: boolean | null;
  billing_period_start_at?: number | null;
  billing_period_end_at?: number | null;
  trial_started_at?: number | null;
  trial_end_at?: number | null;
  token_spend_used_cents?: number | null;
  token_spend_limit_cents?: number | null;
  token_spend_remaining_cents?: number | null;
  edit_predictions_used?: number | null;
  edit_predictions_limit_raw?: string | null;
  edit_predictions_remaining_raw?: string | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;
  usage_updated_at?: number | null;
  spending_limit_cents?: number | null;
  billing_portal_url?: string | null;
  tags?: string[] | null;
  user_raw?: unknown;
  subscription_raw?: unknown;
  usage_raw?: unknown;
  usage_tokens_raw?: unknown;
  preferences_raw?: unknown;
  created_at: number;
  last_used: number;
  email?: string | null;
  plan_type?: string | null;
  quota?: unknown;
}

export interface ZedRuntimeStatus {
  running: boolean;
  lastPid?: number | null;
  lastStartedAt?: number | null;
  currentAccountId?: string | null;
  appPathConfigured: boolean;
}

export interface ZedOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

type ProviderUsage = {
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  allowanceResetAt?: number | null;
  remainingCompletions?: number | null;
  totalCompletions?: number | null;
  remainingChat?: number | null;
  totalChat?: number | null;
};

export type ZedEditPredictionsMetrics = {
  used: number;
  total: number;
  left: number;
  usedPercent: number;
};

function parseFiniteNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const parsed = Number(value.trim());
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function isUnlimitedEditPredictionsLimit(value: string | null | undefined): boolean {
  return value?.trim().toLowerCase() === 'unlimited';
}

export function getZedAccountDisplayEmail(account: ZedAccount): string {
  return account.display_name || account.github_login || account.user_id || account.id;
}

export function getZedPlanBadge(account: ZedAccount): string {
  const raw = account.plan_raw?.trim();
  if (!raw) return 'UNKNOWN';

  const normalized = raw.replace(/^zed_/i, '').trim();
  return normalized ? normalized.toUpperCase() : 'UNKNOWN';
}

export function getZedUsage(account: ZedAccount): ProviderUsage {
  const tokenUsed = parseFiniteNumber(account.token_spend_used_cents);
  const tokenLimit = parseFiniteNumber(account.token_spend_limit_cents);
  const hasEditPredictions =
    account.edit_predictions_used != null || Boolean(account.edit_predictions_limit_raw?.trim());
  const editMetrics = hasEditPredictions ? getZedEditPredictionsMetrics(account) : null;

  const tokenPercent =
    tokenUsed != null && tokenLimit != null && tokenLimit > 0
      ? Math.max(0, Math.min(100, (tokenUsed / tokenLimit) * 100))
      : null;

  return {
    inlineSuggestionsUsedPercent: tokenPercent,
    chatMessagesUsedPercent: editMetrics ? editMetrics.usedPercent : null,
    allowanceResetAt: account.billing_period_end_at != null ? account.billing_period_end_at * 1000 : null,
    remainingCompletions: parseFiniteNumber(account.token_spend_remaining_cents),
    totalCompletions: tokenLimit,
    remainingChat: editMetrics ? editMetrics.left : null,
    totalChat: editMetrics ? editMetrics.total : null,
  };
}

export function formatCurrencyCents(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) return '--';
  return `$${(value / 100).toFixed(2)}`;
}

export function getZedTokenSpendLabel(account: ZedAccount): string {
  return `${formatCurrencyCents(account.token_spend_used_cents)} / ${formatCurrencyCents(account.token_spend_limit_cents)}`;
}

export function getZedEditPredictionsMetrics(account: ZedAccount): ZedEditPredictionsMetrics {
  if (isUnlimitedEditPredictionsLimit(account.edit_predictions_limit_raw)) {
    return {
      used: 0,
      total: 0,
      left: 0,
      usedPercent: 0,
    };
  }

  const used = Math.max(0, parseFiniteNumber(account.edit_predictions_used) ?? 0);
  const total = Math.max(0, parseFiniteNumber(account.edit_predictions_limit_raw) ?? 0);
  const remaining = parseFiniteNumber(account.edit_predictions_remaining_raw);
  const left = Math.max(0, remaining ?? (total > 0 ? total - used : 0));
  const usedPercent =
    total > 0 ? Math.max(0, Math.min(100, (used / total) * 100)) : 0;

  return {
    used,
    total,
    left,
    usedPercent,
  };
}

export function getZedEditPredictionsLabel(account: ZedAccount): string {
  const metrics = getZedEditPredictionsMetrics(account);
  return `${metrics.used} / ${metrics.total}`;
}

export function hasZedQuotaData(account: ZedAccount): boolean {
  return (
    account.usage_raw != null ||
    account.usage_tokens_raw != null ||
    account.token_spend_used_cents != null ||
    account.token_spend_limit_cents != null ||
    account.token_spend_remaining_cents != null ||
    account.edit_predictions_used != null ||
    Boolean(account.edit_predictions_limit_raw?.trim())
  );
}
