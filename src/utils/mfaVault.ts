import * as OTPAuth from 'otpauth';

export interface MfaRecord {
  id: string;
  accountName: string;
  secret: string;
  remark?: string;
  time: number;
}

export interface ParsedMfaCredential {
  accountName: string;
  secret: string;
}

export const MFA_STORAGE_KEY_SAVED = 'agtools.mfa.vault.v2';
export const MFA_STORAGE_KEY_HISTORY = 'agtools.2fa.query.history.v1';

const LEGACY_STORAGE_KEY_SAVED_MFA = 'agtools.mfa.vault.v1';
const LEGACY_STORAGE_KEY_SAVED_2FA = 'agtools.two_factor_auth.saved.v2';
const LEGACY_STORAGE_KEY_HISTORY_2FA = 'agtools.two_factor_auth.history.v2';
const MAX_HISTORY = 50;

export function createMfaRecordId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `mfa-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

export function normalizeStrictBase32(raw: string): string | null {
  const cleaned = raw.trim().replace(/[\s-]/g, '').toUpperCase();
  if (!cleaned) return null;
  if (!/^[A-Z2-7]+=*$/.test(cleaned)) return null;
  return cleaned;
}

function buildAccountDisplayName(issuer: string, label: string): string {
  const issuerPart = issuer.trim();
  const labelPart = label.trim();
  if (issuerPart && labelPart) {
    const lowerIssuer = issuerPart.toLowerCase();
    if (labelPart.toLowerCase().startsWith(`${lowerIssuer}:`)) return labelPart;
    return `${issuerPart}:${labelPart}`;
  }
  return labelPart || issuerPart;
}

function extractSecretFromOtpAuthUri(rawInput: string): string | null {
  const match = rawInput.match(/(?:^|[?&])secret=([^&\s]+)/i);
  if (!match?.[1]) return null;
  try {
    return decodeURIComponent(match[1]).trim();
  } catch {
    return match[1].trim();
  }
}

export function toMfaSecretIdentity(secret: string): string {
  const normalized = normalizeStrictBase32(secret);
  return normalized || secret.trim().toUpperCase();
}

export function parseMfaCredentialInput(rawInput: string): ParsedMfaCredential | null {
  const input = rawInput.trim();
  if (!input) return null;

  try {
    const parsed = OTPAuth.URI.parse(input);
    if (parsed instanceof OTPAuth.TOTP) {
      const rawSecret = extractSecretFromOtpAuthUri(input) || parsed.secret?.base32 || '';
      const validated = normalizeStrictBase32(rawSecret);
      if (!validated) return null;
      const accountName = buildAccountDisplayName(parsed.issuer || '', parsed.label || '');
      return {
        accountName,
        secret: rawSecret,
      };
    }
  } catch {}

  const validated = normalizeStrictBase32(input);
  if (!validated) return null;

  return {
    accountName: '',
    secret: input,
  };
}

function parseAlgorithm(raw: string): 'SHA1' | 'SHA256' | 'SHA512' | undefined {
  const upper = raw.toUpperCase();
  if (upper === 'SHA256') return 'SHA256';
  if (upper === 'SHA512') return 'SHA512';
  if (upper === 'SHA1') return 'SHA1';
  return undefined;
}

function buildMfaTotp(rawSecret: string): OTPAuth.TOTP | null {
  const raw = rawSecret.trim();
  if (!raw) return null;

  try {
    const parsed = OTPAuth.URI.parse(raw);
    if (parsed instanceof OTPAuth.TOTP) return parsed;
  } catch {}

  const secretMatch = raw.match(/(?:^|[?&])secret=([^&\s]+)/i);
  if (secretMatch?.[1]) {
    const secretPart = normalizeStrictBase32(decodeURIComponent(secretMatch[1]));
    if (secretPart) {
      const periodMatch = raw.match(/(?:^|[?&])period=(\d+)/i);
      const digitsMatch = raw.match(/(?:^|[?&])digits=(\d+)/i);
      const algorithmMatch = raw.match(/(?:^|[?&])algorithm=([^&\s]+)/i);
      const period = Number(periodMatch?.[1] || 30);
      const digits = Number(digitsMatch?.[1] || 6);
      const algorithm = parseAlgorithm(decodeURIComponent(algorithmMatch?.[1] || ''));

      try {
        return new OTPAuth.TOTP({
          secret: OTPAuth.Secret.fromBase32(secretPart),
          period: Number.isFinite(period) && period > 0 ? period : 30,
          digits: Number.isFinite(digits) && digits > 0 ? digits : 6,
          algorithm,
        });
      } catch {}
    }
  }

  const normalized = normalizeStrictBase32(raw);
  if (!normalized) return null;
  try {
    return new OTPAuth.TOTP({
      secret: OTPAuth.Secret.fromBase32(normalized),
      period: 30,
      digits: 6,
    });
  } catch {
    return null;
  }
}

export function getMfaOtpToken(secret: string): string {
  const totp = buildMfaTotp(secret);
  if (!totp) return '';
  try {
    return totp.generate();
  } catch {
    return '';
  }
}

export function getMfaTimeRemaining(): number {
  const now = Math.floor(Date.now() / 1000);
  const value = 30 - (now % 30);
  return value === 0 ? 30 : value;
}

function readStorageArray(key: string): unknown[] {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function normalizeMfaRecord(raw: unknown): MfaRecord | null {
  if (!raw || typeof raw !== 'object') return null;
  const item = raw as Record<string, unknown>;
  const sourceSecret = typeof item.secret === 'string' ? item.secret : '';
  const parsed = parseMfaCredentialInput(sourceSecret);
  if (!parsed) return null;

  const accountNameRaw = typeof item.accountName === 'string' ? item.accountName.trim() : '';
  const timeRaw = Number(item.time ?? item.createdAt ?? Date.now());

  return {
    id: createMfaRecordId(),
    accountName: accountNameRaw || parsed.accountName,
    secret: parsed.secret,
    remark: typeof item.remark === 'string' ? item.remark : '',
    time: Number.isFinite(timeRaw) ? timeRaw : Date.now(),
  };
}

export function dedupeMfaRecordsBySecret(records: MfaRecord[]): MfaRecord[] {
  const sorted = [...records].sort((a, b) => b.time - a.time);
  const map = new Map<string, MfaRecord>();
  for (const record of sorted) {
    const identity = toMfaSecretIdentity(record.secret);
    if (!map.has(identity)) {
      map.set(identity, record);
    }
  }
  return Array.from(map.values());
}

export function loadSavedMfaRecords(): MfaRecord[] {
  const merged = [
    ...readStorageArray(MFA_STORAGE_KEY_SAVED),
    ...readStorageArray(LEGACY_STORAGE_KEY_SAVED_MFA),
    ...readStorageArray(LEGACY_STORAGE_KEY_SAVED_2FA),
  ]
    .map(normalizeMfaRecord)
    .filter((item): item is MfaRecord => !!item);

  return dedupeMfaRecordsBySecret(merged);
}

export function loadMfaHistoryRecords(): MfaRecord[] {
  const merged = [
    ...readStorageArray(MFA_STORAGE_KEY_HISTORY),
    ...readStorageArray(LEGACY_STORAGE_KEY_HISTORY_2FA),
  ]
    .map(normalizeMfaRecord)
    .filter((item): item is MfaRecord => !!item);

  return dedupeMfaRecordsBySecret(merged).slice(0, MAX_HISTORY);
}
