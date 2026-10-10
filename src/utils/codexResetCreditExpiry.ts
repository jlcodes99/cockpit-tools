import type { CodexResetCreditsSnapshot } from '../types/codex';

export const RESET_CREDIT_URGENT_SECONDS = 12 * 60 * 60;

/** Prefer individual unconsumed credits; the summary may become stale over time. */
export function getResetCreditExpiry(snapshot: CodexResetCreditsSnapshot, now: number) {
  if (snapshot.available_count === 0) return null;
  const candidates = snapshot.credits
    .filter(credit => !['redeemed', 'used', 'consumed', 'expired'].includes(
      (credit.status || credit.raw_status || 'available').trim().toLowerCase(),
    ))
    .map(credit => credit.expires_at)
    .filter((value): value is number => typeof value === 'number' && Number.isFinite(value) && value > 0);
  const future = candidates.filter(value => value > now);
  const summary = snapshot.next_expires_at;
  const expiresAt = future.length ? Math.min(...future)
    : candidates.length ? Math.max(...candidates)
    : typeof summary === 'number' && Number.isFinite(summary) && summary > 0 ? summary : null;
  if (expiresAt === null) return null;
  const remaining = expiresAt - now;
  return { expiresAt, remaining, expired: remaining <= 0,
    urgent: remaining > 0 && remaining < RESET_CREDIT_URGENT_SECONDS };
}
