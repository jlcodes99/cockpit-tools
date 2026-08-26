import {
  isCodexApiKeyAccount,
  isCodexNewApiAccount,
  isStandardCodexOAuthAccount,
  type CodexAccount,
} from '../types/codex.ts';

export type CodexAutoRefreshPlanKey =
  | 'free'
  | 'go'
  | 'plus'
  | 'pro'
  | 'team'
  | 'business'
  | 'enterprise'
  | 'edu_k12'
  | 'unknown';

export const ALL_CODEX_AUTO_REFRESH_PLAN_KEYS: readonly CodexAutoRefreshPlanKey[] = [
  'free',
  'go',
  'plus',
  'pro',
  'team',
  'business',
  'enterprise',
  'edu_k12',
  'unknown',
];

export interface CodexAutoRefreshPlanOption {
  key: CodexAutoRefreshPlanKey;
  label: string;
  count: number;
}

const CODEX_AUTO_REFRESH_PLAN_LABELS: Record<CodexAutoRefreshPlanKey, string> = {
  free: 'Free',
  go: 'Go',
  plus: 'Plus',
  pro: 'Pro',
  team: 'Team',
  business: 'Business',
  enterprise: 'Enterprise',
  edu_k12: 'Edu / K12',
  unknown: 'Other / Unknown',
};

export function sanitizeCodexAutoRefreshPlanKeys(
  values: readonly string[] | null | undefined,
): CodexAutoRefreshPlanKey[] {
  if (values == null) {
    return [...ALL_CODEX_AUTO_REFRESH_PLAN_KEYS];
  }

  const allowed = new Set<string>(ALL_CODEX_AUTO_REFRESH_PLAN_KEYS);
  const selected = new Set(values.filter((value) => allowed.has(value)));
  return ALL_CODEX_AUTO_REFRESH_PLAN_KEYS.filter((key) => selected.has(key));
}

export function resolveCodexAutoRefreshPlanKey(
  planType?: string | null,
): CodexAutoRefreshPlanKey {
  const normalized = (planType || '').trim().toLowerCase();
  if (!normalized) return 'unknown';

  const words = new Set(normalized.split(/[^a-z0-9]+/).filter(Boolean));
  if (words.has('enterprise')) return 'enterprise';
  if (words.has('business')) return 'business';
  if (words.has('team')) return 'team';
  if (words.has('edu') || words.has('education') || words.has('k12')) return 'edu_k12';
  if (words.has('go')) return 'go';
  if (words.has('plus')) return 'plus';
  if (words.has('pro') || words.has('prolite') || words.has('promax')) return 'pro';
  if (words.has('free')) return 'free';
  return 'unknown';
}

export function isCodexPlanEnabledForAutoRefresh(
  account: Pick<CodexAccount, 'plan_type'>,
  selectedPlanTypes: readonly string[] | null | undefined,
): boolean {
  const selected = new Set(sanitizeCodexAutoRefreshPlanKeys(selectedPlanTypes));
  return selected.has(resolveCodexAutoRefreshPlanKey(account.plan_type));
}

export function isCodexAccountEligibleForAutomaticQuotaRefresh(
  account: CodexAccount,
  selectedPlanTypes: readonly string[] | null | undefined,
): boolean {
  return createCodexAutomaticQuotaRefreshPredicate(selectedPlanTypes)(account);
}

export function createCodexAutomaticQuotaRefreshPredicate(
  selectedPlanTypes: readonly string[] | null | undefined,
): (account: CodexAccount) => boolean {
  const selected = new Set(sanitizeCodexAutoRefreshPlanKeys(selectedPlanTypes));

  return (account) => {
  // Cockpit/New API 账号沿用原有刷新链路；普通 API Key 用量由专用任务处理。
    if (isCodexNewApiAccount(account)) return true;
    if (isCodexApiKeyAccount(account)) return false;

    // 套餐筛选仅约束标准 OAuth 会员账号，避免影响 Web Session 等独立账号类型。
    if (!isStandardCodexOAuthAccount(account)) return true;
    return selected.has(resolveCodexAutoRefreshPlanKey(account.plan_type));
  };
}

export function buildCodexAutoRefreshPlanOptions(
  accounts: readonly Pick<CodexAccount, 'plan_type'>[],
): CodexAutoRefreshPlanOption[] {
  const counts = Object.fromEntries(
    ALL_CODEX_AUTO_REFRESH_PLAN_KEYS.map((key) => [key, 0]),
  ) as Record<CodexAutoRefreshPlanKey, number>;

  for (const account of accounts) {
    counts[resolveCodexAutoRefreshPlanKey(account.plan_type)] += 1;
  }

  return ALL_CODEX_AUTO_REFRESH_PLAN_KEYS.map((key) => ({
    key,
    label: CODEX_AUTO_REFRESH_PLAN_LABELS[key],
    count: counts[key],
  }));
}
