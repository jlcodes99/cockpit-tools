import * as OTPAuth from 'otpauth';
import { Root } from 'protobufjs/light.js';

export interface MfaRecord {
  id: string;
  accountName: string;
  secret: string;
  remark?: string;
  time: number;
}

export type MfaSortField = 'accountName' | 'time';
export type MfaSortDirection = 'asc' | 'desc';
export type MfaSortState = {
  field: MfaSortField;
  direction: MfaSortDirection;
};

export interface ParsedMfaCredential {
  accountName: string;
  secret: string;
}

export interface GoogleAuthenticatorMigrationBatch {
  batchId: number;
  batchIndex: number;
  batchSize: number;
  credentials: ParsedMfaCredential[];
}

export const MFA_STORAGE_KEY_SAVED = 'agtools.mfa.vault.v2';
export const MFA_STORAGE_KEY_HISTORY = 'agtools.2fa.query.history.v1';

const LEGACY_STORAGE_KEY_SAVED_MFA = 'agtools.mfa.vault.v1';
const LEGACY_STORAGE_KEY_SAVED_2FA = 'agtools.two_factor_auth.saved.v2';
const LEGACY_STORAGE_KEY_HISTORY_2FA = 'agtools.two_factor_auth.history.v2';

interface GoogleAuthenticatorMigrationOtp {
  secret?: Uint8Array;
  name?: string;
  issuer?: string;
  type?: number;
}

interface GoogleAuthenticatorMigrationPayload {
  otpParameters?: GoogleAuthenticatorMigrationOtp[];
  batchSize?: number;
  batchIndex?: number;
  batchId?: number;
}

const googleAuthenticatorMigrationPayloadType = Root.fromJSON({
  nested: {
    MigrationPayload: {
      fields: {
        otpParameters: { rule: 'repeated', type: 'OtpParameters', id: 1 },
        version: { type: 'int32', id: 2 },
        batchSize: { type: 'int32', id: 3 },
        batchIndex: { type: 'int32', id: 4 },
        batchId: { type: 'int32', id: 5 },
      },
    },
    OtpParameters: {
      fields: {
        secret: { type: 'bytes', id: 1 },
        name: { type: 'string', id: 2 },
        issuer: { type: 'string', id: 3 },
        algorithm: { type: 'int32', id: 4 },
        digits: { type: 'int32', id: 5 },
        type: { type: 'int32', id: 6 },
        counter: { type: 'int64', id: 7 },
      },
    },
  },
}).lookupType('MigrationPayload');

function decodeBase64Url(value: string): Uint8Array | null {
  try {
    const normalized = value.replace(/-/g, '+').replace(/_/g, '/');
    const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, '=');
    const binary = atob(padded);
    return Uint8Array.from(binary, char => char.charCodeAt(0));
  } catch {
    return null;
  }
}

function encodeBase32(bytes: Uint8Array): string {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let buffer = 0;
  let bits = 0;
  let result = '';

  for (const byte of bytes) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      result += alphabet[(buffer >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }

  if (bits > 0) result += alphabet[(buffer << (5 - bits)) & 31];
  return result;
}

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

export function splitMfaAccountName(raw: string): { issuer: string; account: string } {
  const name = raw.trim();
  const separator = name.indexOf(':');
  if (separator > 0 && name.slice(separator + 1).trim()) {
    return { issuer: name.slice(0, separator).trim(), account: name.slice(separator + 1).trim() };
  }
  return { issuer: '', account: name };
}

export function filterMfaRecords(records: readonly MfaRecord[], query: string): MfaRecord[] {
  const term = query.trim().toLocaleLowerCase();
  return records.filter(record => record.accountName.toLocaleLowerCase().includes(term));
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

export function parseGoogleAuthenticatorMigrationBatch(rawInput: string): GoogleAuthenticatorMigrationBatch | null {
  const input = rawInput.trim();
  if (!input.startsWith('otpauth-migration://')) return null;

  try {
    const uri = new URL(input);
    if (uri.protocol !== 'otpauth-migration:' || uri.hostname !== 'offline') return null;

    const encodedPayload = uri.searchParams.get('data');
    if (!encodedPayload) return null;

    const bytes = decodeBase64Url(encodedPayload);
    if (!bytes) return null;

    const decoded = googleAuthenticatorMigrationPayloadType.decode(bytes) as unknown as GoogleAuthenticatorMigrationPayload;
    const credentials = (decoded.otpParameters || [])
      .filter(item => item.type === 2 && item.secret && item.secret.length > 0)
      .map(item => ({
        accountName: buildAccountDisplayName(item.issuer || '', item.name || ''),
        secret: encodeBase32(item.secret!),
      }))
      .filter(item => !!normalizeStrictBase32(item.secret));

    return {
      batchId: Number(decoded.batchId) || 0,
      batchIndex: Math.max(0, Number(decoded.batchIndex) || 0),
      batchSize: Math.max(1, Number(decoded.batchSize) || 1),
      credentials,
    };
  } catch {
    return null;
  }
}

export function parseGoogleAuthenticatorMigrationInput(rawInput: string): ParsedMfaCredential[] | null {
  const isMigrationInput = rawInput.trim().startsWith('otpauth-migration://');
  const batch = parseGoogleAuthenticatorMigrationBatch(rawInput);
  if (batch) return batch.credentials;
  return isMigrationInput ? [] : null;
}

export function parseMfaCredentialInputs(rawInput: string): ParsedMfaCredential[] {
  const input = rawInput.trim();
  if (!input) return [];

  const migrationCredentials = parseGoogleAuthenticatorMigrationInput(input);
  if (migrationCredentials) return migrationCredentials;

  try {
    const parsed = OTPAuth.URI.parse(input);
    if (parsed instanceof OTPAuth.TOTP) {
      const rawSecret = extractSecretFromOtpAuthUri(input) || parsed.secret?.base32 || '';
      const validated = normalizeStrictBase32(rawSecret);
      if (!validated) return [];
      const accountName = buildAccountDisplayName(parsed.issuer || '', parsed.label || '');
      return [{
        accountName,
        secret: rawSecret,
      }];
    }
  } catch {}

  const validated = normalizeStrictBase32(input);
  if (!validated) return [];

  return [{
    accountName: '',
    secret: input,
  }];
}

export function parseMfaCredentialInput(rawInput: string): ParsedMfaCredential | null {
  return parseMfaCredentialInputs(rawInput)[0] || null;
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

export function exportGoogleAuthenticatorMigrationUris(
  records: readonly MfaRecord[],
): Array<{ uri: string; count: number }> {
  if (records.length === 0) return [];
  const batchId = crypto.getRandomValues(new Uint32Array(1))[0] & 0x7fffffff;
  const entries = records.map(record => {
    if (record.accountName.length > 2000 || record.secret.length > 2000) {
      throw new Error('mfaVault.migrationRecordTooLarge');
    }
    let totp: OTPAuth.TOTP | null = null;
    try {
      const rawSecret = record.secret.trim();
      if (rawSecret.startsWith('otpauth:')) {
        const parsed = OTPAuth.URI.parse(rawSecret);
        if (parsed instanceof OTPAuth.TOTP) totp = parsed;
      } else if (normalizeStrictBase32(rawSecret)) {
        totp = buildMfaTotp(rawSecret);
      }
    } catch { totp = null; }
    if (!totp || totp.secret.bytes.length === 0) throw new Error('mfaVault.migrationInvalidRecord');
    // The existing vault stores 30-second, six-digit SHA1 credentials.
    // Reject settings the vault cannot round-trip instead of changing their codes.
    if (totp.algorithm !== 'SHA1' || totp.digits !== 6 || totp.period !== 30) {
      throw new Error('mfaVault.migrationUnsupportedParameters');
    }
    const { issuer } = splitMfaAccountName(record.accountName);
    return { secret: totp.secret.bytes, name: record.accountName.trim(), issuer, algorithm: 1, digits: 1, type: 2 };
  });
  const encode = (otpParameters: typeof entries, batchIndex: number, batchSize: number) => {
    const payload = googleAuthenticatorMigrationPayloadType.encode({
      otpParameters, version: 1, batchIndex, batchSize, batchId,
    }).finish();
    const data = btoa(Array.from(payload, byte => String.fromCharCode(byte)).join(''));
    return `otpauth-migration://offline?${new URLSearchParams({ data })}`;
  };
  const batches: Array<typeof entries> = [];
  let batch: typeof entries = [];
  for (const entry of entries) {
    // Reserve the largest metadata values before final batch counts are known.
    if (encode([entry], entries.length, entries.length).length > 1500) {
      throw new Error('mfaVault.migrationRecordTooLarge');
    }
    const candidate = [...batch, entry];
    if (candidate.length > 10 || encode(candidate, entries.length, entries.length).length > 1500) {
      batches.push(batch);
      batch = [entry];
    } else {
      batch = candidate;
    }
  }
  if (batch.length) batches.push(batch);
  return batches.map((items, index) => ({ uri: encode(items, index, batches.length), count: items.length }));
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

const mfaAccountNameCollator = new Intl.Collator('zh-CN', {
  numeric: true,
  sensitivity: 'base',
});

export function sortMfaRecords(records: readonly MfaRecord[], sort: MfaSortState): MfaRecord[] {
  const direction = sort.direction === 'asc' ? 1 : -1;
  return records
    .map((record, index) => ({ record, index, accountName: record.accountName.trim() }))
    .sort((a, b) => {
      if (sort.field === 'accountName' && !!a.accountName !== !!b.accountName) {
        return a.accountName ? -1 : 1;
      }

      const nameComparison = mfaAccountNameCollator.compare(a.accountName, b.accountName);
      const timeComparison = a.record.time - b.record.time;
      const comparison = sort.field === 'accountName'
        ? nameComparison || timeComparison
        : timeComparison || nameComparison;
      return (comparison || a.index - b.index) * direction;
    })
    .map(({ record }) => record);
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

  return dedupeMfaRecordsBySecret(merged);
}

export function rememberMfaQuery(input: {
  secret: string;
  accountName?: string | null;
  remark?: string | null;
}): MfaRecord[] {
  const parsed = parseMfaCredentialInput(input.secret);
  if (!parsed?.secret || !getMfaOtpToken(input.secret)) {
    return loadMfaHistoryRecords();
  }

  const now = Date.now();
  const records = loadMfaHistoryRecords();
  const identity = toMfaSecretIdentity(parsed.secret);
  const existing = records.find(
    (record) => toMfaSecretIdentity(record.secret) === identity,
  );
  const accountName = input.accountName?.trim() || parsed.accountName || '';
  const remark = input.remark?.trim() || '';
  const nextRecord: MfaRecord = existing
    ? {
        ...existing,
        secret: parsed.secret,
        accountName: accountName || existing.accountName,
        remark: remark || existing.remark,
        time: now,
      }
    : {
        id: createMfaRecordId(),
        secret: parsed.secret,
        accountName,
        remark,
        time: now,
      };
  const nextRecords = dedupeMfaRecordsBySecret([
    nextRecord,
    ...records.filter(
      (record) => toMfaSecretIdentity(record.secret) !== identity,
    ),
  ]);

  try {
    localStorage.setItem(MFA_STORAGE_KEY_HISTORY, JSON.stringify(nextRecords));
  } catch {
    // Local 2FA query history sync is best effort.
  }
  return nextRecords;
}

export function upsertSavedMfaRecord(input: {
  secret: string;
  accountName?: string | null;
  remark?: string | null;
}): MfaRecord[] {
  const parsed = parseMfaCredentialInput(input.secret);
  if (!parsed?.secret) {
    return loadSavedMfaRecords();
  }

  const now = Date.now();
  const records = loadSavedMfaRecords();
  const identity = toMfaSecretIdentity(parsed.secret);
  const existingIndex = records.findIndex(
    (record) => toMfaSecretIdentity(record.secret) === identity,
  );
  const accountName = input.accountName?.trim() || parsed.accountName || "";
  const remark = input.remark?.trim() || "";

  if (existingIndex >= 0) {
    records[existingIndex] = {
      ...records[existingIndex],
      secret: parsed.secret,
      accountName: accountName || records[existingIndex].accountName,
      remark: remark || records[existingIndex].remark,
      time: now,
    };
  } else {
    records.unshift({
      id: createMfaRecordId(),
      secret: parsed.secret,
      accountName,
      remark,
      time: now,
    });
  }

  const nextRecords = dedupeMfaRecordsBySecret(records);
  try {
    localStorage.setItem(MFA_STORAGE_KEY_SAVED, JSON.stringify(nextRecords));
  } catch {
    // Local 2FA vault sync is best effort.
  }
  return nextRecords;
}
