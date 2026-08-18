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
  type: 'oauth' | 'apikey';
  credentials: JsonRecord;
  extra?: JsonRecord;
  concurrency: number;
  priority: number;
  expires_at?: number;
  auto_pause_on_expired?: boolean;
}

const SUB2API_OPENAI_CLIENT_ID = 'app_EMoamEEZ73f0CkXaXp7hrann';
const SUB2API_DEFAULT_OPENAI_BASE_URL = 'https://api.openai.com';

interface CodexPortableTokenStorage extends JsonRecord {
  id_token: string;
  access_token: string;
  refresh_token: string;
  account_id: string;
  last_refresh: string;
  email: string;
  type: 'codex';
  expired: string;
  account_note?: string;
  two_factor_secret?: string;
  account_password?: string;
  phone_number?: string;
  mail_url?: string;
}

interface CodexPortableAgentIdentityStorage extends JsonRecord {
  auth_mode: 'agentIdentity';
  agent_identity: JsonRecord;
  account_id: string;
  user_id: string;
  email: string;
  type: 'codex';
}

interface CodexExportBuildOptions {
  includeSensitiveNotes?: boolean;
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

function appendSensitiveNoteFields(target: JsonRecord, account: CodexAccount): void {
  const accountNote = account.account_note?.trim();
  if (accountNote) {
    target.account_note = accountNote;
  }

  const twoFactorSecret = account.two_factor_secret?.trim();
  if (twoFactorSecret) {
    target.two_factor_secret = twoFactorSecret;
  }

  const accountPassword = account.account_password?.trim();
  if (accountPassword) {
    target.account_password = accountPassword;
  }

  const phoneNumber = account.phone_number?.trim();
  if (phoneNumber) {
    target.phone_number = phoneNumber;
  }

  const mailUrl = account.mail_url?.trim();
  if (mailUrl) {
    target.mail_url = mailUrl;
  }
}

function hasSensitiveNoteFields(account: CodexAccount): boolean {
  return Boolean(
    account.account_note?.trim() ||
      account.two_factor_secret?.trim() ||
      account.account_password?.trim() ||
      account.phone_number?.trim() ||
      account.mail_url?.trim(),
  );
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

function resolveAccessAuthPayload(account: CodexAccount): JsonRecord | null {
  const accessTokenPayload = decodeJwtPayload(account.tokens?.access_token);
  return toJsonRecord(accessTokenPayload?.['https://api.openai.com/auth']);
}

function resolveAuthProvider(account: CodexAccount): string | undefined {
  const idTokenPayload = decodeJwtPayload(account.tokens?.id_token);
  return toStringValue(idTokenPayload?.auth_provider);
}

function resolveAccountId(account: CodexAccount): string | undefined {
  const authPayload = resolveAuthPayload(account);
  const accessAuthPayload = resolveAccessAuthPayload(account);
  return (
    toStringValue(account.account_id) ||
    toStringValue(authPayload?.chatgpt_account_id) ||
    toStringValue(accessAuthPayload?.chatgpt_account_id) ||
    toStringValue(authPayload?.account_id) ||
    toStringValue(accessAuthPayload?.account_id)
  );
}

function resolveUserId(account: CodexAccount): string | undefined {
  const idTokenPayload = decodeJwtPayload(account.tokens?.id_token);
  const authPayload = resolveAuthPayload(account);
  const accessAuthPayload = resolveAccessAuthPayload(account);
  return (
    toStringValue(account.user_id) ||
    toStringValue(authPayload?.chatgpt_user_id) ||
    toStringValue(accessAuthPayload?.chatgpt_user_id) ||
    toStringValue(authPayload?.user_id) ||
    toStringValue(accessAuthPayload?.user_id) ||
    toStringValue(idTokenPayload?.sub)
  );
}

function resolveOrganizationId(account: CodexAccount): string | undefined {
  const authPayload = resolveAuthPayload(account);
  const accessAuthPayload = resolveAccessAuthPayload(account);
  return (
    toStringValue(account.organization_id) ||
    toStringValue(authPayload?.organization_id) ||
    toStringValue(accessAuthPayload?.organization_id) ||
    toStringValue(authPayload?.poid) ||
    toStringValue(accessAuthPayload?.poid)
  );
}

function resolvePlanType(account: CodexAccount): string | undefined {
  const authPayload = resolveAuthPayload(account);
  const accessAuthPayload = resolveAccessAuthPayload(account);
  return (
    toStringValue(account.plan_type) ||
    toStringValue(authPayload?.chatgpt_plan_type) ||
    toStringValue(accessAuthPayload?.chatgpt_plan_type)
  );
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
  return normalizeTimestampToIso(account.subscription_active_until);
}

function resolveAccessTokenExpiry(account: CodexAccount): string | undefined {
  const accessTokenPayload = decodeJwtPayload(account.tokens?.access_token);
  const accessExp = toNumberValue(accessTokenPayload?.exp);
  return normalizeTimestampToIso(accessExp);
}

function resolveLastRefresh(account: CodexAccount): string {
  return normalizeTimestampToIso(account.token_updated_at) || new Date().toISOString();
}

function hasAgentIdentity(account: CodexAccount): boolean {
  return Boolean(account.agent_identity?.agent_runtime_id?.trim());
}

function buildAgentIdentityCredentials(account: CodexAccount): JsonRecord {
  const identity = account.agent_identity;
  const agentRuntimeId = identity?.agent_runtime_id?.trim();
  const agentPrivateKey = identity?.agent_private_key?.trim();
  const accountId = identity?.account_id?.trim();
  const chatgptUserId = identity?.chatgpt_user_id?.trim();
  if (!agentRuntimeId || !agentPrivateKey || !accountId || !chatgptUserId) {
    throw new Error('Codex Agent Identity credentials are incomplete');
  }

  const credentials: JsonRecord = {
    auth_mode: 'agentIdentity',
    agent_runtime_id: agentRuntimeId,
    agent_private_key: agentPrivateKey,
    account_id: accountId,
    chatgpt_account_id: accountId,
    chatgpt_user_id: chatgptUserId,
    chatgpt_account_is_fedramp:
      identity?.chatgpt_account_is_fedramp === true,
  };
  const taskId = identity?.task_id?.trim();
  if (taskId) {
    credentials.task_id = taskId;
  }
  const email = identity?.email?.trim() || account.email?.trim();
  if (email) {
    credentials.email = email;
  }
  const planType = identity?.plan_type?.trim() || account.plan_type?.trim();
  if (planType) {
    credentials.plan_type = planType;
  }
  return credentials;
}

function buildSub2apiCredentials(account: CodexAccount): JsonRecord {
  if (hasAgentIdentity(account)) {
    return buildAgentIdentityCredentials(account);
  }

  const credentials: JsonRecord = {
    access_token: account.tokens.access_token,
  };

  const expiresAt = resolveAccessTokenExpiry(account);
  if (expiresAt) {
    credentials.expires_at = expiresAt;
  }

  if (account.tokens.refresh_token?.trim()) {
    credentials.refresh_token = account.tokens.refresh_token.trim();
    credentials.client_id = SUB2API_OPENAI_CLIENT_ID;
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

function buildSub2apiApiKeyCredentials(account: CodexAccount): JsonRecord {
  const apiKey = account.openai_api_key?.trim();
  if (!apiKey) {
    throw new Error('SUB2API_API_KEY_MISSING');
  }
  return {
    base_url: account.api_base_url?.trim() || SUB2API_DEFAULT_OPENAI_BASE_URL,
    api_key: apiKey,
  };
}

function buildSub2apiExtra(account: CodexAccount): JsonRecord | undefined {
  const authProvider = resolveAuthProvider(account);
  const extra: JsonRecord = {};
  if (authProvider) {
    extra.auth_provider = authProvider;
  }
  if (
    account.codex_fingerprint_mode &&
    account.codex_fingerprint_mode !== 'off'
  ) {
    extra.codex_fingerprint_mode = account.codex_fingerprint_mode;
  }
  return Object.keys(extra).length > 0 ? extra : undefined;
}

function toSub2apiAccount(account: CodexAccount): Sub2apiCreateAccountItem {
  const base = {
    name: account.account_name?.trim() || account.email || account.id,
    platform: 'openai' as const,
    concurrency: 3,
    priority: 50,
  };

  if (isCodexApiKeyAccount(account)) {
    return {
      ...base,
      type: 'apikey',
      credentials: buildSub2apiApiKeyCredentials(account),
    };
  }

  if (hasAgentIdentity(account)) {
    return {
      ...base,
      type: 'oauth',
      credentials: buildSub2apiCredentials(account),
    };
  }
  if (!account.tokens.access_token?.trim()) {
    throw new Error('SUB2API_ACCESS_TOKEN_MISSING');
  }

  const credentials = buildSub2apiCredentials(account);
  const extra = buildSub2apiExtra(account);
  const item: Sub2apiCreateAccountItem = {
    ...base,
    type: 'oauth',
    credentials,
    ...(extra ? { extra } : {}),
  };

  if (!account.tokens.refresh_token?.trim()) {
    const tokenExpiresAt = resolveAccessTokenExpiry(account);
    if (!tokenExpiresAt) {
      throw new Error('SUB2API_ACCESS_TOKEN_EXPIRY_MISSING');
    }
    item.expires_at = Math.floor(new Date(tokenExpiresAt).getTime() / 1000);
    item.auto_pause_on_expired = true;
  }

  return item;
}

function toPortableTokenStorage(
  account: CodexAccount,
  options: CodexExportBuildOptions = {},
): CodexPortableTokenStorage {
  const payload: CodexPortableTokenStorage = {
    id_token: account.tokens.id_token || '',
    access_token: account.tokens.access_token || '',
    refresh_token: account.tokens.refresh_token?.trim() || '',
    account_id: resolveAccountId(account) || '',
    last_refresh: resolveLastRefresh(account),
    email: account.email || '',
    type: 'codex',
    expired: resolveAccessTokenExpiry(account) || '',
  };
  if (account.codex_fingerprint_mode) {
    payload.codex_fingerprint_mode = account.codex_fingerprint_mode;
  }

  if (options.includeSensitiveNotes) {
    appendSensitiveNoteFields(payload, account);
  }

  return payload;
}

function toPortableAgentIdentityStorage(
  account: CodexAccount,
  options: CodexExportBuildOptions = {},
): CodexPortableAgentIdentityStorage {
  const credentials = buildAgentIdentityCredentials(account);
  const payload: CodexPortableAgentIdentityStorage = {
    auth_mode: 'agentIdentity',
    agent_identity: credentials,
    account_id: String(credentials.account_id),
    user_id: String(credentials.chatgpt_user_id),
    email: String(credentials.email || account.email || ''),
    type: 'codex',
  };

  const planType = toStringValue(credentials.plan_type);
  if (planType) {
    payload.plan_type = planType;
  }
  if (account.account_name?.trim()) {
    payload.account_name = account.account_name.trim();
  }
  if (account.account_structure?.trim()) {
    payload.account_structure = account.account_structure.trim();
  }
  if (options.includeSensitiveNotes) {
    appendSensitiveNoteFields(payload, account);
  }
  return payload;
}

function isCodexApiKeyAccount(account: CodexAccount): boolean {
  return account.auth_mode === 'apikey' || Boolean(account.openai_api_key?.trim());
}

function toPortableApiKeyStorage(
  account: CodexAccount,
  options: CodexExportBuildOptions = {},
): JsonRecord {
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

  if (options.includeSensitiveNotes) {
    appendSensitiveNoteFields(payload, account);
  }

  return payload;
}

function toCockpitToolsPortableStorage(
  account: CodexAccount,
  options: CodexExportBuildOptions = {},
): CodexPortableTokenStorage | JsonRecord {
  if (hasAgentIdentity(account)) {
    return toPortableAgentIdentityStorage(account, options);
  }
  if (isCodexApiKeyAccount(account)) {
    return toPortableApiKeyStorage(account, options);
  }
  return toPortableTokenStorage(account, options);
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

export function hasCodexExportSensitiveNotes(rawJson: string): boolean {
  try {
    return parseCockpitToolsCodexExport(rawJson).some(hasSensitiveNoteFields);
  } catch {
    return false;
  }
}

export function hasCodexExportAgentIdentity(rawJson: string): boolean {
  try {
    return parseCockpitToolsCodexExport(rawJson).some(hasAgentIdentity);
  } catch {
    return false;
  }
}

export function transformCodexExportJson(
  rawJson: string,
  format: CodexExportFormat,
  options: CodexExportBuildOptions = {},
): string {
  const accounts = parseCockpitToolsCodexExport(rawJson);

  if (format === 'cockpit_tools') {
    return JSON.stringify(
      accounts.map((account) => toCockpitToolsPortableStorage(account, options)),
      null,
      2,
    );
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

  if (accounts.some(hasAgentIdentity)) {
    throw new Error('CPA format does not support Codex Agent Identity accounts');
  }

  const cpaPayload = accounts.map((account) => toPortableTokenStorage(account, options));
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
  options: CodexExportBuildOptions = {},
): CodexExportContent {
  const fileNameBase = buildCodexExportFileNameBase(baseName, format);
  const accounts = parseCockpitToolsCodexExport(rawJson);

  if (format !== 'cpa' || accounts.length <= 1) {
    return {
      type: 'single',
      fileNameBase,
      jsonContent: transformCodexExportJson(rawJson, format, options),
    };
  }

  return {
    type: 'multiple',
    fileNameBase,
    documents: accounts.map((account, index) => ({
      id: `${account.id || resolveAccountId(account) || 'cpa_account'}_${index}`,
      label: resolveCpaDocumentLabel(account, index),
      fileNameBase: buildCpaDocumentFileNameBase(fileNameBase, account, index),
      jsonContent: JSON.stringify(toPortableTokenStorage(account, options), null, 2),
    })),
  };
}
