export type ClaudeAuthMode =
  | 'oauth'
  | 'o_auth'
  | 'setup_token'
  | 'api_key'
  | 'desktop_oauth'
  | 'desktop_o_auth'
  | 'desktop_gateway';

export type NormalizedClaudeAuthMode = 'oauth' | 'setup_token' | 'api_key' | 'desktop_oauth' | 'desktop_gateway';

export interface ClaudeQuotaErrorInfo {
  code?: string | null;
  message: string;
  timestamp: number;
}

export interface ClaudeQuota {
  five_hour_percentage: number;
  five_hour_reset_time?: number | null;
  seven_day_percentage: number;
  seven_day_reset_time?: number | null;
  seven_day_sonnet_percentage?: number | null;
  seven_day_sonnet_reset_time?: number | null;
  extra_usage_percentage?: number | null;
  extra_usage_reset_time?: number | null;
  extra_usage_used_cents?: number | null;
  extra_usage_limit_cents?: number | null;
  raw_data?: unknown;
}

export interface ClaudeAccount {
  id: string;
  email: string;
  auth_mode?: ClaudeAuthMode;
  account_uuid?: string | null;
  organization_uuid?: string | null;
  organization_name?: string | null;
  plan_type?: string | null;
  avatar_url?: string | null;
  profile_updated_at?: number | null;
  quota?: ClaudeQuota | null;
  quota_error?: ClaudeQuotaErrorInfo | null;
  usage_updated_at?: number | null;
  status?: string | null;
  status_reason?: string | null;
  api_key?: string | null;
  api_base_url?: string | null;
  api_provider_id?: string | null;
  api_provider_name?: string | null;
  api_provider_source_tag?: string | null;
  api_provider_website?: string | null;
  api_provider_api_key_url?: string | null;
  api_key_field?: string | null;
  api_model_catalog?: string[] | null;
  api_extra_env?: Record<string, string> | null;
  desktop_gateway_auth_scheme?: string | null;
  desktop_gateway_credential_kind?: string | null;
  desktop_gateway_config_id?: string | null;
  desktop_gateway_profile_dir?: string | null;
  desktop_gateway_models?: string[] | null;
  desktop_gateway_connection_mode?: ClaudeDesktopGatewayConnectionMode | string | null;
  desktop_gateway_upstream_models?: string[] | null;
  desktop_gateway_model_mappings?: ClaudeDesktopGatewayModelMapping[] | null;
  desktop_profile_dir?: string | null;
  desktop_profile_imported_at?: number | null;
  claude_credentials_raw?: unknown;
  claude_config_raw?: unknown;
  claude_usage_raw?: unknown;
  tags?: string[] | null;
  account_note?: string | null;
  created_at: number;
  last_used: number;
}

export interface ClaudeDesktopLoginStartResponse {
  loginId: string;
  userDataDir: string;
  expiresIn: number;
  intervalSeconds: number;
}

export type ClaudeDesktopLoginProgressPhase =
  | 'start'
  | 'profile'
  | 'resolve-runtime'
  | 'check-cache'
  | 'cached'
  | 'download-start'
  | 'downloading'
  | 'downloaded'
  | 'verify'
  | 'extract'
  | 'runtime-ready'
  | 'launch'
  | 'ready';

export interface ClaudeDesktopLoginProgressPayload {
  progressId: string;
  phase: ClaudeDesktopLoginProgressPhase | string;
  percent?: number | null;
  downloadedBytes?: number | null;
  totalBytes?: number | null;
}

export interface ClaudeOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
}

export interface ClaudeDesktopGatewayModel {
  id: string;
  displayName?: string | null;
}

export type ClaudeDesktopGatewayConnectionMode = 'direct' | 'local_mapping';

export interface ClaudeDesktopGatewayModelMapping {
  desktopModel: string;
  upstreamModel: string;
  labelOverride?: string | null;
  supports1m?: boolean | null;
}

export interface ClaudeDesktopGatewayModelsResult {
  models: ClaudeDesktopGatewayModel[];
  latencyMs: number;
  recommendedMode?: ClaudeDesktopGatewayConnectionMode | string | null;
  hasClaudeModels?: boolean;
}

function asPlainRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function readStringPath(value: unknown, path: string[]): string {
  let current: unknown = value;
  for (const key of path) {
    const record = asPlainRecord(current);
    if (!record) return '';
    current = record[key];
  }
  return typeof current === 'string' ? current.trim() : '';
}

function readBooleanPath(value: unknown, path: string[]): boolean {
  let current: unknown = value;
  for (const key of path) {
    const record = asPlainRecord(current);
    if (!record) return false;
    current = record[key];
  }
  return current === true;
}

function normalizeClaudePlanValue(value?: string | null): string {
  const raw = value?.trim();
  if (!raw) return '';
  const key = raw.toLowerCase().replace(/[-_]+/g, ' ').replace(/\s+/g, ' ').trim();
  switch (key) {
    case 'default claude max 20x':
    case 'claude max 20x':
    case 'max 20x':
      return 'Max 20x';
    case 'default claude max 5x':
    case 'claude max 5x':
    case 'max 5x':
      return 'Max 5x';
    case 'claude max':
    case 'max':
      return 'Max';
    case 'claude pro':
    case 'pro':
      return 'Pro';
    case 'default claude ai':
    case 'claude free':
    case 'free':
      return 'Free';
    case 'claude enterprise':
    case 'enterprise':
      return 'Enterprise';
    case 'claude team':
    case 'team':
      return 'Team';
    default:
      return '';
  }
}

function isClaudeBillingSourceValue(value?: string | null): boolean {
  const key = value?.trim().toLowerCase().replace(/[-_]+/g, ' ').replace(/\s+/g, ' ');
  return key === 'apple subscription' || key === 'stripe subscription';
}

function deriveClaudePlanBadge(account: ClaudeAccount): string {
  const configOauth = asPlainRecord(asPlainRecord(account.claude_config_raw)?.oauthAccount);
  const credentialsOauth = asPlainRecord(asPlainRecord(account.claude_credentials_raw)?.claudeAiOauth);
  const profile = asPlainRecord(credentialsOauth?.profile);

  const candidates = [
    account.plan_type,
    readStringPath(configOauth, ['subscriptionType']),
    readStringPath(credentialsOauth, ['subscriptionType']),
    readStringPath(configOauth, ['organizationType']),
    readStringPath(profile, ['organization', 'organization_type']),
    readStringPath(configOauth, ['rateLimitTier']),
    readStringPath(credentialsOauth, ['rateLimitTier']),
    readStringPath(profile, ['organization', 'rate_limit_tier']),
  ];

  for (const candidate of candidates) {
    const normalized = normalizeClaudePlanValue(candidate);
    if (normalized) return normalized;
  }

  if (readBooleanPath(profile, ['account', 'has_claude_max'])) return 'Max';
  if (readBooleanPath(profile, ['account', 'has_claude_pro'])) return 'Pro';

  return '';
}

export function normalizeClaudeAuthMode(mode?: ClaudeAuthMode | string | null): NormalizedClaudeAuthMode {
  if (mode === 'desktop_oauth' || mode === 'desktop_o_auth') return 'desktop_oauth';
  if (mode === 'desktop_gateway') return 'desktop_gateway';
  if (mode === 'api_key') return 'api_key';
  if (mode === 'setup_token') return 'setup_token';
  return 'oauth';
}

export function isClaudeDesktopOAuthAccount(account: ClaudeAccount): boolean {
  if (normalizeClaudeAuthMode(account.auth_mode) === 'desktop_gateway') return false;
  if (normalizeClaudeAuthMode(account.auth_mode) === 'desktop_oauth') return true;
  if (account.desktop_profile_dir || account.desktop_profile_imported_at) return true;
  const configRaw = asPlainRecord(account.claude_config_raw);
  return Boolean(asPlainRecord(configRaw?.desktopProfile));
}

export function isClaudeDesktopGatewayAccount(account: ClaudeAccount): boolean {
  return normalizeClaudeAuthMode(account.auth_mode) === 'desktop_gateway';
}

export function isClaudeDesktopRuntimeAccount(account: ClaudeAccount): boolean {
  return isClaudeDesktopGatewayAccount(account) || isClaudeDesktopOAuthAccount(account);
}

export function getClaudeAccountDisplayEmail(account: ClaudeAccount): string {
  const email = account.email?.trim();
  if (email) return email;
  const org = account.organization_name?.trim();
  if (org) return org;
  return account.id;
}

export function getClaudePlanBadge(account: ClaudeAccount): string {
  const derivedPlan = deriveClaudePlanBadge(account);
  if (derivedPlan) return derivedPlan;

  const raw = account.plan_type?.trim();
  if (isClaudeDesktopOAuthAccount(account)) {
    if (!raw || /^claude\s+desktop$/i.test(raw) || /^desktop$/i.test(raw) || isClaudeBillingSourceValue(raw)) return '';
    return raw;
  }
  if (raw && !isClaudeBillingSourceValue(raw)) return raw;
  if (!raw) {
    const org = account.organization_name?.trim();
    if (org) return org;
  }
  return 'Personal';
}

export function getClaudeAuthModeLabel(account: ClaudeAccount): string {
  const normalizedMode = isClaudeDesktopGatewayAccount(account)
    ? 'desktop_gateway'
    : isClaudeDesktopOAuthAccount(account)
      ? 'desktop_oauth'
      : normalizeClaudeAuthMode(account.auth_mode);
  switch (normalizedMode) {
    case 'desktop_gateway':
      return 'Claude Gateway';
    case 'desktop_oauth':
      return 'Claude';
    case 'api_key':
      return 'API Key';
    case 'setup_token':
      return 'Setup Token';
    case 'oauth':
    default:
      return 'OAuth';
  }
}

export function getClaudeApiProviderLabel(account: ClaudeAccount): string {
  const providerName = account.api_provider_name?.trim();
  if (providerName) return providerName;
  const baseUrl = account.api_base_url?.trim();
  if (!baseUrl) return '';
  try {
    return new URL(baseUrl).hostname.replace(/^www\./, '');
  } catch {
    return baseUrl;
  }
}

export function getClaudePlanBadgeClass(account: ClaudeAccount): string {
  const raw = getClaudePlanBadge(account).toLowerCase();
  if (!raw) return 'unknown';
  if (raw.includes('team') || raw.includes('enterprise') || raw.includes('org')) {
    return 'team';
  }
  if (raw.includes('free')) return 'free';
  if (raw.includes('max 20') || raw.includes('20x')) return 'ultra';
  if (raw.includes('max')) return 'pro';
  if (raw.includes('pro')) return 'pro';
  return 'free';
}

export function getClaudeQuotaClass(usedPercent: number): 'high' | 'medium' | 'low' | 'critical' {
  if (usedPercent >= 90) return 'critical';
  if (usedPercent >= 70) return 'low';
  if (usedPercent >= 40) return 'medium';
  return 'high';
}

export function getClaudeRemainingPercentage(usedPercent: number): number {
  if (!Number.isFinite(usedPercent)) return 0;
  return 100 - Math.max(0, Math.min(100, Math.round(usedPercent)));
}

export function getClaudeRemainingQuotaClass(
  remainingPercent: number,
): 'high' | 'medium' | 'low' | 'critical' {
  const normalized = Number.isFinite(remainingPercent)
    ? Math.max(0, Math.min(100, Math.round(remainingPercent)))
    : 0;
  return getClaudeQuotaClass(100 - normalized);
}

export function hasClaudeQuotaData(account: ClaudeAccount): boolean {
  return Boolean(account.quota);
}

export function getClaudeUsage(account: ClaudeAccount): {
  totalPercentUsed: number | null;
  fiveHourPercentUsed: number | null;
  sevenDayPercentUsed: number | null;
  sevenDaySonnetPercentUsed: number | null;
} {
  const fiveHour = account.quota?.five_hour_percentage;
  const sevenDay = account.quota?.seven_day_percentage;
  const values = [fiveHour, sevenDay].filter(
    (value): value is number => typeof value === 'number' && Number.isFinite(value),
  );
  return {
    totalPercentUsed: values.length > 0 ? Math.max(...values) : null,
    fiveHourPercentUsed: typeof fiveHour === 'number' ? fiveHour : null,
    sevenDayPercentUsed: typeof sevenDay === 'number' ? sevenDay : null,
    sevenDaySonnetPercentUsed: null,
  };
}

export function formatClaudeResetTime(value?: number | null): string {
  if (!value || !Number.isFinite(value)) return '';
  const seconds = value > 10_000_000_000 ? Math.floor(value / 1000) : Math.floor(value);
  return new Date(seconds * 1000).toLocaleString();
}
