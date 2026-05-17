import type { CodexAccount } from '../types/codex';

export type CodexExportFormat = 'cockpit_tools' | 'sub2api' | 'cpa';

type JsonRecord = Record<string, unknown>;
const INVALID_FILE_CHARS_REGEX = /[<>:"/\\|?*\x00-\x1F]/g;

interface Sub2apiBatchCreatePayload {
  exported_at: string;
  proxies: [];
  accounts: Sub2apiCreateAccountItem[];
  type: 'sub2api-data';
  version: 1;
}

interface Sub2apiCreateAccountItem {
  name: string;
  platform: 'openai';
  type: 'oauth';
  credentials: JsonRecord;
  concurrency: number;
  priority: number;
}

interface CodexPortableTokenStorage {
  id_token: string;
  access_token: string;
  refresh_token: string;
  account_id: string;
  last_refresh: string;
  email: string;
  type: 'codex';
  expired: string;
}

export interface CodexExportDocument {
  id: string;
  label: string;
  fileNameBase: string;
  jsonContent: string;
}

export type CodexExportContent =
  | {
      type: 'single';
      fileNameBase: string;
      jsonContent: string;
    }
  | {
      type: 'multiple';
      fileNameBase: string;
      documents: CodexExportDocument[];
    };

function toJsonRecord(value: unknown): JsonRecord | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as JsonRecord) : null;
}

function toStringValue(value: unknown): string | undefined {
  if (typeof value === 'string') {
    const trimmed = value.trim();
    return trimmed || undefined;
  }
  if (typeof value === 'number' && Number.isFinite(value)) {
    return String(value);
  }
  return undefined;
}

function toNumberValue(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

function sanitizeFileNameSegment(input: string | undefined, fallback: string): string {
  const raw = (input || '').trim();
  const normalized = raw
    .replace(INVALID_FILE_CHARS_REGEX, '_')
    .replace(/\s+/g, '_')
    .replace(/_+/g, '_')
    .replace(/^_+|_+$/g, '');
  return normalized || fallback;
}

function decodeJwtPayload(token: string | undefined): JsonRecord | null {
  if (!token) return null;
  const parts = token.split('.');
  if (parts.length < 2) return null;

  const payloadPart = parts[1];
  const padded = payloadPart + '='.repeat((4 - (payloadPart.length % 4)) % 4);
  const base64 = padded.replace(/-/g, '+').replace(/_/g, '/');

  try {
    const binary = atob(base64);
    const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
    const text = new TextDecoder().decode(bytes);
    return toJsonRecord(JSON.parse(text));
  } catch {
    return null;
  }
}

function resolveAuthPayload(account: CodexAccount): JsonRecord | null {
  const idTokenPayload = decodeJwtPayload(account.tokens?.id_token);
  return toJsonRecord(idTokenPayload?.['https://api.openai.com/auth']);
}

function resolveAccountId(account: CodexAccount): string | undefined {
  const authPayload = resolveAuthPayload(account);
  return (
    toStringValue(account.account_id) ||
    toStringValue(authPayload?.chatgpt_account_id) ||
    toStringValue(authPayload?.account_id)
  );
}

function resolveUserId(account: CodexAccount): string | undefined {
  const idTokenPayload = decodeJwtPayload(account.tokens?.id_token);
  const authPayload = resolveAuthPayload(account);
  return (
    toStringValue(account.user_id) ||
    toStringValue(authPayload?.chatgpt_user_id) ||
    toStringValue(authPayload?.user_id) ||
    toStringValue(idTokenPayload?.sub)
  );
}

function resolveOrganizationId(account: CodexAccount): string | undefined {
  const authPayload = resolveAuthPayload(account);
  return toStringValue(account.organization_id) || toStringValue(authPayload?.organization_id);
}

function resolvePlanType(account: CodexAccount): string | undefined {
  const authPayload = resolveAuthPayload(account);
  return toStringValue(account.plan_type) || toStringValue(authPayload?.chatgpt_plan_type);
}

function normalizeTimestampToIso(value: unknown): string | undefined {
  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (!trimmed) return undefined;
    const parsed = Date.parse(trimmed);
    return Number.isFinite(parsed) ? new Date(parsed).toISOString() : trimmed;
  }

  const numeric = toNumberValue(value);
  if (numeric == null) return undefined;
  const millis = numeric > 1_000_000_000_000 ? numeric : numeric * 1000;
  const date = new Date(millis);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

function formatSub2apiExportedAt(): string {
  return new Date().toISOString().replace(/\.\d{3}Z$/, 'Z');
}

function resolveSubscriptionExpiresAt(account: CodexAccount): string | undefined {
  const authPayload = resolveAuthPayload(account);
  return (
    normalizeTimestampToIso(account.subscription_active_until) ||
    normalizeTimestampToIso(authPayload?.chatgpt_subscription_active_until)
  );
}

function resolveAccessTokenExpiry(account: CodexAccount): string | undefined {
  const accessTokenPayload = decodeJwtPayload(account.tokens?.access_token);
  const idTokenPayload = decodeJwtPayload(account.tokens?.id_token);
  const accessExp = toNumberValue(accessTokenPayload?.exp);
  if (accessExp != null) {
    return normalizeTimestampToIso(accessExp);
  }
  const idExp = toNumberValue(idTokenPayload?.exp);
  return normalizeTimestampToIso(idExp);
}

function resolveLastRefresh(account: CodexAccount): string {
  return normalizeTimestampToIso(account.token_updated_at) || new Date().toISOString();
}

function buildSub2apiCredentials(account: CodexAccount): JsonRecord {
  const credentials: JsonRecord = {
    access_token: account.tokens.access_token,
  };

  const expiresAt = resolveAccessTokenExpiry(account);
  if (expiresAt) {
    credentials.expires_at = expiresAt;
  }

  if (account.tokens.refresh_token?.trim()) {
    credentials.refresh_token = account.tokens.refresh_token.trim();
  }
  if (account.tokens.id_token?.trim()) {
    credentials.id_token = account.tokens.id_token.trim();
  }
  if (account.email?.trim()) {
    credentials.email = account.email.trim();
  }

  const chatgptAccountId = resolveAccountId(account);
  if (chatgptAccountId) {
    credentials.chatgpt_account_id = chatgptAccountId;
  }

  const chatgptUserId = resolveUserId(account);
  if (chatgptUserId) {
    credentials.chatgpt_user_id = chatgptUserId;
  }

  const organizationId = resolveOrganizationId(account);
  if (organizationId) {
    credentials.organization_id = organizationId;
  }

  const planType = resolvePlanType(account);
  if (planType) {
    credentials.plan_type = planType;
  }

  const subscriptionExpiresAt = resolveSubscriptionExpiresAt(account);
  if (subscriptionExpiresAt) {
    credentials.subscription_expires_at = subscriptionExpiresAt;
  }

  return credentials;
}

function toSub2apiAccount(account: CodexAccount): Sub2apiCreateAccountItem {
  return {
    name: account.account_name?.trim() || account.email || account.id,
    platform: 'openai',
    type: 'oauth',
    credentials: buildSub2apiCredentials(account),
    concurrency: 0,
    priority: 0,
  };
}

function toPortableTokenStorage(account: CodexAccount): CodexPortableTokenStorage {
  return {
    id_token: account.tokens.id_token || '',
    access_token: account.tokens.access_token || '',
    refresh_token: account.tokens.refresh_token?.trim() || '',
    account_id: resolveAccountId(account) || '',
    last_refresh: resolveLastRefresh(account),
    email: account.email || '',
    type: 'codex',
    expired: resolveAccessTokenExpiry(account) || '',
  };
}

function isCodexApiKeyAccount(account: CodexAccount): boolean {
  return account.auth_mode === 'apikey' || Boolean(account.openai_api_key?.trim());
}

function toPortableApiKeyStorage(account: CodexAccount): JsonRecord {
  const payload: JsonRecord = {
    auth_mode: 'apikey',
    OPENAI_API_KEY: account.openai_api_key || '',
    email: account.email || '',
  };

  if (account.api_base_url?.trim()) {
    payload.api_base_url = account.api_base_url.trim();
  }
  if (account.api_provider_id?.trim()) {
    payload.api_provider_id = account.api_provider_id.trim();
  }
  if (account.api_provider_name?.trim()) {
    payload.api_provider_name = account.api_provider_name.trim();
  }

  return payload;
}

function toCockpitToolsPortableStorage(account: CodexAccount): CodexPortableTokenStorage | JsonRecord {
  if (isCodexApiKeyAccount(account)) {
    return toPortableApiKeyStorage(account);
  }
  return toPortableTokenStorage(account);
}

export function parseCockpitToolsCodexExport(rawJson: string): CodexAccount[] {
  const parsed = JSON.parse(rawJson) as unknown;
  if (Array.isArray(parsed)) {
    return parsed as CodexAccount[];
  }
  if (parsed && typeof parsed === 'object') {
    return [parsed as CodexAccount];
  }
  return [];
}

export function transformCodexExportJson(
  rawJson: string,
  format: CodexExportFormat,
): string {
  const accounts = parseCockpitToolsCodexExport(rawJson);

  if (format === 'cockpit_tools') {
    return JSON.stringify(accounts.map(toCockpitToolsPortableStorage), null, 2);
  }

  if (format === 'sub2api') {
    const payload: Sub2apiBatchCreatePayload = {
      exported_at: formatSub2apiExportedAt(),
      proxies: [],
      accounts: accounts.map(toSub2apiAccount),
      type: 'sub2api-data',
      version: 1,
    };
    return JSON.stringify(payload, null, 2);
  }

  const cpaPayload = accounts.map(toPortableTokenStorage);
  const normalizedPayload = cpaPayload.length === 1 ? cpaPayload[0] : cpaPayload;
  return JSON.stringify(normalizedPayload, null, 2);
}

export function buildCodexExportFileNameBase(
  baseName: string,
  format: CodexExportFormat,
): string {
  if (format === 'cockpit_tools') {
    return baseName;
  }
  return `${baseName}_${format}`;
}

function resolveCpaDocumentLabel(account: CodexAccount, index: number): string {
  return (
    account.email?.trim() ||
    resolveAccountId(account) ||
    account.account_name?.trim() ||
    account.id ||
    `account_${index + 1}`
  );
}

function buildCpaDocumentFileNameBase(
  baseName: string,
  account: CodexAccount,
  index: number,
): string {
  const label = sanitizeFileNameSegment(
    account.email?.trim() || resolveAccountId(account) || account.id,
    `account_${index + 1}`,
  );
  const accountIdSuffix = sanitizeFileNameSegment(resolveAccountId(account), '');
  const suffix =
    accountIdSuffix && accountIdSuffix !== label ? `_${accountIdSuffix.slice(-6)}` : '';
  return `${baseName}_${String(index + 1).padStart(2, '0')}_${label}${suffix}`;
}

export function buildCodexExportContent(
  rawJson: string,
  format: CodexExportFormat,
  baseName: string,
): CodexExportContent {
  const fileNameBase = buildCodexExportFileNameBase(baseName, format);
  const accounts = parseCockpitToolsCodexExport(rawJson);

  if (format !== 'cpa' || accounts.length <= 1) {
    return {
      type: 'single',
      fileNameBase,
      jsonContent: transformCodexExportJson(rawJson, format),
    };
  }

  return {
    type: 'multiple',
    fileNameBase,
    documents: accounts.map((account, index) => ({
      id: `${account.id || resolveAccountId(account) || 'cpa_account'}_${index}`,
      label: resolveCpaDocumentLabel(account, index),
      fileNameBase: buildCpaDocumentFileNameBase(fileNameBase, account, index),
      jsonContent: JSON.stringify(toPortableTokenStorage(account), null, 2),
    })),
  };
}
