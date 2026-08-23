import type { MfaRecord } from './mfaVault';

export const ACCOUNT_CONVERSION_CHALLENGE_TYPES = [
  'password_current',
  'password_new',
  'totp',
  'recovery_email_code',
  'phone_code',
  'passkey',
  'authenticator_setup',
  'backup_codes',
  'phone_removal',
  'session_signout',
  'captcha',
  'account_recovery',
  'extension_install',
  'generic',
] as const;

export type AccountConversionChallengeType =
  (typeof ACCOUNT_CONVERSION_CHALLENGE_TYPES)[number];

export type AccountConversionChallengeStatus =
  | 'queued'
  | 'presented'
  | 'user_confirmed'
  | 'cancelled'
  | 'expired';

export const ACCOUNT_CONVERSION_STATUS_LABELS: Record<
  AccountConversionChallengeStatus,
  string
> = {
  queued: '等待处理',
  presented: '正在处理',
  user_confirmed: '用户已确认',
  cancelled: '已取消',
  expired: '已过期',
};

export interface AccountConversionChallenge {
  id: string;
  batchId: string;
  runId: string;
  slot: string;
  port: number;
  chromePid: number;
  expectedEmail: string;
  type: AccountConversionChallengeType;
  instructions: string;
  status: AccountConversionChallengeStatus;
  createdAt: string;
  updatedAt: string;
  expiresAt: string;
  presentedAt: string | null;
  confirmedAt: string | null;
}

export interface AccountConversionBridgeStatus {
  running: boolean;
  schemaVersion: number;
  capabilities: string[];
  baseUrl: string | null;
  pid: number;
  startedAt: string | null;
  queuedCount: number;
  presentedCount: number;
}

const FULL_EMAIL_PATTERN = /[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi;

export function emailsFromMfaLabel(label: string): string[] {
  return [...new Set(
    (label.match(FULL_EMAIL_PATTERN) ?? []).map((email) => email.toLowerCase()),
  )];
}

/**
 * Matches only a complete email observed in the non-secret MFA label/remark.
 * The TOTP secret is intentionally never used as an account identity key.
 */
export function matchingMfaRecords(
  records: MfaRecord[],
  expectedEmail: string,
): MfaRecord[] {
  const expected = expectedEmail.trim().toLowerCase();
  if (!expected) return [];
  return records.filter((record) => {
    const labels = [record.accountName, record.remark ?? ''];
    return labels.some((label) => emailsFromMfaLabel(label).includes(expected));
  });
}

export function conversionChallengeNeedsMfaSelection(
  type: AccountConversionChallengeType,
): boolean {
  return type === 'totp' || type === 'authenticator_setup';
}

export function canConfirmConversionChallenge(
  type: AccountConversionChallengeType,
  selectedMfaKey: string,
): boolean {
  return !conversionChallengeNeedsMfaSelection(type) || Boolean(selectedMfaKey);
}

export function isActiveConversionChallenge(
  challenge: AccountConversionChallenge,
): boolean {
  return challenge.status === 'queued' || challenge.status === 'presented';
}

export function challengeStatusLabel(
  status: AccountConversionChallengeStatus,
): string {
  return ACCOUNT_CONVERSION_STATUS_LABELS[status];
}

export function challengeExpiresInMinutes(
  challenge: Pick<AccountConversionChallenge, 'expiresAt'>,
  nowMs = Date.now(),
): number {
  const expiresAt = Date.parse(challenge.expiresAt);
  if (!Number.isFinite(expiresAt)) return 0;
  return Math.max(0, Math.ceil((expiresAt - nowMs) / 60_000));
}
