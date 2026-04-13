export type CodexApiProviderMode = 'openai_builtin' | 'custom';

export interface CodexQuickConfig {
  context_window_1m: boolean;
  auto_compact_token_limit: number;
  detected_model_context_window?: number;
  detected_auto_compact_token_limit?: number;
}

/** Codex 账号数据 */
export interface CodexAccount {
  id: string;
  email: string;
  auth_mode?: string;
  openai_api_key?: string;
  api_base_url?: string;
  api_provider_mode?: CodexApiProviderMode;
  api_provider_id?: string;
  api_provider_name?: string;
  user_id?: string;
  plan_type?: string;
  account_id?: string;
  organization_id?: string;
  account_name?: string;
  account_structure?: string;
  tokens: CodexTokens;
  quota?: CodexQuota;
  quota_error?: CodexQuotaErrorInfo;
  tags?: string[];
  created_at: number;
  last_used: number;
}

export interface CodexQuotaErrorInfo {
  code?: string;
  message: string;
  timestamp: number;
}

/** Codex Token 数据 */
export interface CodexTokens {
  id_token: string;
  access_token: string;
  refresh_token?: string;
}

/** Codex 配额数据 */
export interface CodexQuota {
  /** 5小时配额百分比 (0-100) */
  hourly_percentage: number;
  /** 5小时配额重置时间 (Unix timestamp) */
  hourly_reset_time?: number;
  /** 主窗口时长（分钟） */
  hourly_window_minutes?: number;
  /** 主窗口是否存在（接口返回） */
  hourly_window_present?: boolean;
  /** 周配额百分比 (0-100) */
  weekly_percentage: number;
  /** 周配额重置时间 (Unix timestamp) */
  weekly_reset_time?: number;
  /** 次窗口时长（分钟） */
  weekly_window_minutes?: number;
  /** 次窗口是否存在（接口返回） */
  weekly_window_present?: boolean;
  /** 原始响应数据 */
  raw_data?: unknown;
}

export interface CodexWorkspace {
  id: string;
  title: string;
  role?: string;
  is_default?: boolean;
}

export interface CodexAuthMetadata {
  chatgptAccountId?: string;
  authProvider?: string;
  userId?: string;
  workspaces: CodexWorkspace[];
}

export interface CodexCodeReviewQuotaMetric {
  percentage: number;
  label: string;
  resetTime?: number;
}

export interface CodexInstanceThreadSyncItem {
  instanceId: string;
  instanceName: string;
  addedThreadCount: number;
  backupDir?: string | null;
}

export interface CodexInstanceThreadSyncSummary {
  instanceCount: number;
  threadUniverseCount: number;
  mutatedInstanceCount: number;
  totalSyncedThreadCount: number;
  items: CodexInstanceThreadSyncItem[];
  backupDirs: string[];
  message: string;
}

export interface CodexSessionVisibilityRepairItem {
  instanceId: string;
  instanceName: string;
  targetProvider: string;
  changedRolloutFileCount: number;
  updatedSqliteRowCount: number;
  backupDir?: string | null;
  running: boolean;
}

export interface CodexSessionVisibilityRepairSummary {
  instanceCount: number;
  mutatedInstanceCount: number;
  changedRolloutFileCount: number;
  updatedSqliteRowCount: number;
  items: CodexSessionVisibilityRepairItem[];
  backupDirs: string[];
  message: string;
}

export interface CodexSessionLocation {
  instanceId: string;
  instanceName: string;
  running: boolean;
}

export interface CodexSessionRecord {
  sessionId: string;
  title: string;
  cwd: string;
  updatedAt?: number | null;
  locationCount: number;
  locations: CodexSessionLocation[];
}

export interface CodexSessionTrashSummary {
  requestedSessionCount: number;
  trashedSessionCount: number;
  trashedInstanceCount: number;
  trashDirs: string[];
  message: string;
}

export interface CodexTrashedSessionLocation {
  instanceId: string;
  instanceName: string;
}

export interface CodexTrashedSessionRecord {
  sessionId: string;
  title: string;
  cwd: string;
  deletedAt?: number | null;
  locationCount: number;
  locations: CodexTrashedSessionLocation[];
}

export interface CodexSessionRestoreSummary {
  requestedSessionCount: number;
  restoredSessionCount: number;
  restoredInstanceCount: number;
  message: string;
}

type JsonRecord = Record<string, unknown>;

function toJsonRecord(value: unknown): JsonRecord | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as JsonRecord) : null;
}

function toStringValue(value: unknown): string | undefined {
  if (typeof value === 'string') {
    const trimmed = value.trim();
    return trimmed || undefined;
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }
  return undefined;
}

function toBoolValue(value: unknown): boolean | undefined {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') {
    if (value === 1) return true;
    if (value === 0) return false;
  }
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    if (normalized === 'true') return true;
    if (normalized === 'false') return false;
  }
  return undefined;
}

function toFiniteNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
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

function normalizeWorkspaceList(value: unknown): CodexWorkspace[] {
  if (!Array.isArray(value)) return [];
  const dedupe = new Set<string>();
  const result: CodexWorkspace[] = [];

  value.forEach((item) => {
    const record = toJsonRecord(item);
    if (!record) return;
    const id = toStringValue(record.id) || toStringValue(record.organization_id) || toStringValue(record.workspace_id);
    const title =
      toStringValue(record.title) ||
      toStringValue(record.name) ||
      toStringValue(record.display_name) ||
      toStringValue(record.workspace_name) ||
      toStringValue(record.organization_name);
    if (!id && !title) return;
    const dedupeKey = `${id || ''}::${title || ''}`;
    if (dedupe.has(dedupeKey)) return;
    dedupe.add(dedupeKey);
    result.push({
      id: id || '',
      title: title || id || '',
      role: toStringValue(record.role),
      is_default: toBoolValue(record.is_default),
    });
  });

  return result;
}

export function getCodexAuthMetadata(account: CodexAccount): CodexAuthMetadata {
  const idTokenPayload = decodeJwtPayload(account.tokens?.id_token);
  const accessTokenPayload = decodeJwtPayload(account.tokens?.access_token);
  const idTokenAuthData = toJsonRecord(idTokenPayload?.['https://api.openai.com/auth']);
  const accessTokenAuthData = toJsonRecord(accessTokenPayload?.['https://api.openai.com/auth']);

  const chatgptAccountId =
    account.account_id ||
    toStringValue(idTokenAuthData?.chatgpt_account_id) ||
    toStringValue(accessTokenAuthData?.chatgpt_account_id) ||
    toStringValue(idTokenAuthData?.account_id);
  const authProvider = toStringValue(idTokenPayload?.auth_provider);
  const userId =
    account.user_id ||
    toStringValue(idTokenAuthData?.chatgpt_user_id) ||
    toStringValue(accessTokenAuthData?.chatgpt_user_id) ||
    toStringValue(idTokenAuthData?.user_id) ||
    toStringValue(accessTokenAuthData?.user_id) ||
    toStringValue(idTokenPayload?.sub);
  const workspaces = normalizeWorkspaceList(idTokenAuthData?.organizations);

  return {
    chatgptAccountId,
    authProvider,
    userId,
    workspaces,
  };
}

export function formatCodexLoginProvider(rawProvider: string | undefined): string {
  const value = rawProvider?.trim();
  if (!value) return '';
  const normalized = value.toLowerCase();
  if (normalized === 'google') return 'Google';
  if (normalized === 'github') return 'GitHub';
  if (normalized === 'microsoft') return 'Microsoft';
  if (normalized === 'apple') return 'Apple';
  if (normalized === 'password') return 'Password';
  return value;
}

function normalizeCodeReviewWindow(
  window: JsonRecord,
  fallback: 'hourly' | 'weekly',
): CodexCodeReviewQuotaMetric | null {
  const usedPercent = toFiniteNumber(window.used_percent);
  if (usedPercent === undefined) return null;
  const percentage = Math.max(0, Math.min(100, 100 - Math.round(usedPercent)));
  const limitWindowSeconds = toFiniteNumber(window.limit_window_seconds);
  const windowMinutes =
    limitWindowSeconds !== undefined && limitWindowSeconds > 0
      ? Math.ceil(limitWindowSeconds / 60)
      : undefined;
  const resetAt = toFiniteNumber(window.reset_at);
  const resetAfterSeconds = toFiniteNumber(window.reset_after_seconds);
  const resetTime =
    resetAt ??
    (resetAfterSeconds !== undefined && resetAfterSeconds >= 0
      ? Math.floor(Date.now() / 1000) + resetAfterSeconds
      : undefined);

  return {
    percentage,
    label: getCodexQuotaWindowLabel(windowMinutes, fallback),
    resetTime,
  };
}

export function getCodexCodeReviewQuotaMetric(
  quota: CodexQuota | undefined,
): CodexCodeReviewQuotaMetric | null {
  const raw = toJsonRecord(quota?.raw_data);
  const rateLimit = toJsonRecord(raw?.code_review_rate_limit);
  if (!rateLimit) return null;

  const primaryWindow = toJsonRecord(rateLimit.primary_window);
  const secondaryWindow = toJsonRecord(rateLimit.secondary_window);

  return (
    (primaryWindow ? normalizeCodeReviewWindow(primaryWindow, 'hourly') : null) ||
    (secondaryWindow ? normalizeCodeReviewWindow(secondaryWindow, 'weekly') : null)
  );
}

export function isCodexApiKeyAccount(account: CodexAccount): boolean {
  return (account.auth_mode || '').trim().toLowerCase() === 'apikey';
}

/** 获取订阅类型显示名称 */
export function getCodexPlanDisplayName(planType?: string): string {
  if (!planType) return 'FREE';
  const upper = planType.toUpperCase();
  if (upper.includes('TEAM')) return 'TEAM';
  if (upper.includes('ENTERPRISE')) return 'ENTERPRISE';
  if (upper.includes('PLUS')) return 'PLUS';
  if (upper.includes('PRO')) return 'PRO';
  return upper;
}

export function isCodexTeamLikePlan(planType?: string): boolean {
  if (!planType) return false;
  const upper = planType.toUpperCase();
  return (
    upper.includes('TEAM') ||
    upper.includes('BUSINESS') ||
    upper.includes('ENTERPRISE') ||
    upper.includes('EDU')
  );
}

export function hasCodexAccountName(account: CodexAccount): boolean {
  return typeof account.account_name === 'string' && account.account_name.trim().length > 0;
}

export function hasCodexAccountStructure(account: CodexAccount): boolean {
  return (
    typeof account.account_structure === 'string' && account.account_structure.trim().length > 0
  );
}

/** 获取配额百分比的样式类名 */
export function getCodexQuotaClass(percentage: number): string {
  if (percentage >= 80) return 'high';
  if (percentage >= 40) return 'medium';
  if (percentage >= 10) return 'low';
  return 'critical';
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

export interface CodexQuotaWindow {
  id: 'primary' | 'secondary';
  label: string;
  percentage: number;
  resetTime?: number;
  windowMinutes?: number;
}

export function getCodexQuotaWindowLabel(
  windowMinutes: number | undefined,
  fallback: 'hourly' | 'weekly' = 'hourly'
): string {
  const HOUR_MINUTES = 60;
  const DAY_MINUTES = 24 * HOUR_MINUTES;
  const WEEK_MINUTES = 7 * DAY_MINUTES;
  const safeMinutes =
    typeof windowMinutes === 'number' && Number.isFinite(windowMinutes) && windowMinutes > 0
      ? Math.ceil(windowMinutes)
      : null;

  if (safeMinutes == null) {
    return fallback === 'weekly' ? 'Weekly' : '5h';
  }

  if (safeMinutes >= WEEK_MINUTES - 1) {
    const weeks = Math.ceil(safeMinutes / WEEK_MINUTES);
    return weeks <= 1 ? 'Weekly' : `${weeks} Week`;
  }

  if (safeMinutes >= DAY_MINUTES - 1) {
    return `${Math.ceil(safeMinutes / DAY_MINUTES)}d`;
  }

  if (safeMinutes >= HOUR_MINUTES) {
    return `${Math.ceil(safeMinutes / HOUR_MINUTES)}h`;
  }

  return `${Math.ceil(safeMinutes)}m`;
}

export function getCodexQuotaWindows(quota: CodexQuota | undefined): CodexQuotaWindow[] {
  if (!quota) return [];

  const windows: CodexQuotaWindow[] = [];
  const hasPresenceFlags =
    quota.hourly_window_present !== undefined || quota.weekly_window_present !== undefined;

  const appendPrimary = !hasPresenceFlags || quota.hourly_window_present === true;
  const appendSecondary = !hasPresenceFlags || quota.weekly_window_present === true;

  if (appendPrimary) {
    windows.push({
      id: 'primary',
      label: getCodexQuotaWindowLabel(quota.hourly_window_minutes, 'hourly'),
      percentage: quota.hourly_percentage,
      resetTime: quota.hourly_reset_time,
      windowMinutes: quota.hourly_window_minutes,
    });
  }

  if (appendSecondary) {
    windows.push({
      id: 'secondary',
      label: getCodexQuotaWindowLabel(quota.weekly_window_minutes, 'weekly'),
      percentage: quota.weekly_percentage,
      resetTime: quota.weekly_reset_time,
      windowMinutes: quota.weekly_window_minutes,
    });
  }

  if (windows.length > 0) {
    return windows;
  }

  return [
    {
      id: 'primary',
      label: getCodexQuotaWindowLabel(quota.hourly_window_minutes, 'hourly'),
      percentage: quota.hourly_percentage,
      resetTime: quota.hourly_reset_time,
      windowMinutes: quota.hourly_window_minutes,
    },
  ];
}

/** 格式化重置时间显示（相对时间 + 绝对时间） */
export function formatCodexResetTime(
  resetTime: number | undefined,
  t: Translate
): string {
  if (!resetTime) return '';

  const now = Math.floor(Date.now() / 1000);
  const diff = resetTime - now;

  if (diff <= 0) return t('common.shared.quota.resetDone');

  const totalMinutes = Math.floor(diff / 60);
  const days = Math.floor(totalMinutes / (60 * 24));
  const hours = Math.floor((totalMinutes % (60 * 24)) / 60);
  const minutes = totalMinutes % 60;

  let parts = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  if (minutes > 0) parts.push(`${minutes}m`);
  
  const relative = parts.length > 0 ? parts.join(' ') : '<1m';
  const absolute = formatCodexResetTimeAbsolute(resetTime);

  return `${relative} (${absolute})`;
}

export function formatCodexResetTimeAbsolute(
  resetTime: number | undefined
): string {
  if (!resetTime) return '';

  const resetDate = new Date(resetTime * 1000);
  
  const pad = (value: number) => String(value).padStart(2, '0');
  const month = pad(resetDate.getMonth() + 1);
  const day = pad(resetDate.getDate());
  const hours = pad(resetDate.getHours());
  const minutes = pad(resetDate.getMinutes());
  
  return `${month}/${day} ${hours}:${minutes}`;
}
