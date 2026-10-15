import { getCodexPlanBadgePresentation, getCodexQuotaClass, getCodexQuotaWindows, type CodexAccount } from '../../../types/codex.ts';
import { CODEX_PELICAN_MAX_CONCURRENCY } from '../../../types/codexPelican.ts';

export function parsePelicanConcurrency(value: string): number | null {
  if (!/^\d+$/.test(value.trim())) return null;
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= 1 && parsed <= CODEX_PELICAN_MAX_CONCURRENCY ? parsed : null;
}

export function defaultPelicanConcurrency(accountCount: number): number {
  return Math.min(CODEX_PELICAN_MAX_CONCURRENCY, Math.max(1, Math.floor(accountCount)));
}

/** Pro uses the same tier names as overview; other backend plan values stay verbatim. */
export function pelicanAccountSummary(account: CodexAccount) {
  const badge = getCodexPlanBadgePresentation(account, { preserveRawNonProLabel: true });
  return {
    plan: account.plan_type?.trim() || account.auth_file_plan_type?.trim() ? badge.label : null,
    planClass: badge.className,
    windows: getCodexQuotaWindows(account.quota).map((window) => {
      const raw = window.id === 'primary' ? account.quota?.hourly_percentage : account.quota?.weekly_percentage;
      const known = typeof raw === 'number' && Number.isFinite(raw);
      return { ...window, percentage: known ? window.percentage : null,
        quotaClass: known ? getCodexQuotaClass(window.percentage) : 'unknown' };
    }),
  };
}
