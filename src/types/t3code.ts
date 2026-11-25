import type { CodexProviderQuotaDisplay } from '../utils/codexProviderPoolQuota';
import type {
  CodebuddySuiteAccountBase,
  CodebuddyUsage,
  OfficialQuotaResource,
  QuotaCategoryGroup,
} from './codebuddy-suite';

export type T3CodeDriverKind =
  | 'codex'
  | 'antigravity'
  | 'claudeAgent'
  | 'workbuddy'
  | 'workbuddy_ai'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'grok'
  | 'minimaxcode';

export interface T3CodeAccount extends CodebuddySuiteAccountBase {
  user_id?: string;
  display_name?: string;
  avatar_url?: string;
  username?: string;
  auth_mode?: 'clerk' | 'clerk_token' | 'local_profile' | string;
  provider?: string;
  clerk_jwt?: string;
  clerk_client_jwt?: string;
  clerk_session_token?: string;
  clerk_session_id?: string;
  session_id?: string;
  environment_id?: string;
  expires_at?: number;
  plan_type?: string;
  identity_verified_at?: number;
  relay_verified_at?: number;
  t3chat_linked?: boolean;
  quota_total?: number;
  quota_used?: number;
  quota_remaining?: number;
  quota_reset_at?: number;
  quota_updated_at?: number;
  usage_updated_at?: number;
  quota_raw?: unknown;
  usage_raw?: unknown;
  profile_raw?: unknown;
}

export type T3CodeQuotaWindowView = {
  id: string;
  label: string;
  resetTime?: string;
} & (
  | { remainingPercent: number; remainingAmount?: never }
  | { remainingAmount: number; remainingPercent?: never }
);

export interface T3CodeAntigravityQuotaGroup {
  id: string;
  name: string;
  windows: { id: string; period: string; percentage: number }[];
}

export interface T3CodeBindableAccountView {
  id: string;
  driver: T3CodeDriverKind;
  email: string;
  displayName?: string;
  planBadge: string;
  planBadgeKind?: string;
  authMode: string;
  isApiService: boolean;
  workspaceId?: string;
  windows: T3CodeQuotaWindowView[];
  quotaSummaryGroups?: T3CodeAntigravityQuotaGroup[];
  providerQuota?: CodexProviderQuotaDisplay;
  primaryPercent?: number;
  secondaryPercent?: number;
  speedTokPerSec?: number;
  cacheHitPercent?: number;
}

export interface T3CodeProviderRuntimeView {
  instanceId: string;
  installed?: boolean;
  accountEmail?: string;
  workspaceId?: string;
  authStatus?: string;
  authUsesKeyring?: boolean;
  modelCount: number;
  availability?: string;
  matchesGateway: boolean;
  source?: 'live' | 'cache' | string;
  status?: string;
  message?: string;
  checkedAt?: string;
}

export interface T3CodeProviderInstanceView {
  instanceId: string;
  driver: T3CodeDriverKind;
  displayName: string;
  accentColor?: string;
  enabled: boolean;
  isBuiltinDefault: boolean;
  homePath?: string;
  shadowHomePath?: string;
  authMethod?: string;
  boundAccountId?: string;
  boundAccountEmail?: string;
  boundAccountPlan?: string;
  boundIsApiService: boolean;
  boundAccountWorkspaceId?: string;
  boundUpdatedAt?: number;
  bindingAppliedLive?: boolean;
  modelRefreshPending?: boolean;
  pendingDefault?: {
    state: 'waiting' | 'restartRequired' | 'failed';
    error?: string;
  };
  needsRepair?: boolean;
  explicitRuntimePath?: boolean;
  runtime?: T3CodeProviderRuntimeView;
  windows: T3CodeQuotaWindowView[];
  quotaSummaryGroups?: T3CodeAntigravityQuotaGroup[];
  providerQuota?: CodexProviderQuotaDisplay;
  speedTokPerSec?: number;
  cacheHitPercent?: number;
}

export interface T3CodeProviderWorkspaceView {
  t3InstanceId: string;
  stateDir: string;
  t3Running: boolean;
  t3StartedAt?: string;
  injectionConnected: boolean;
  providerInstances: T3CodeProviderInstanceView[];
  codexAccounts: T3CodeBindableAccountView[];
  codexUnverifiedCount?: number;
  antigravityAccounts: T3CodeBindableAccountView[];
  claudeAccounts: T3CodeBindableAccountView[];
  extraAccounts?: Record<string, T3CodeBindableAccountView[]>;
}

export interface T3CodeNativeModel {
  instanceId: string;
  providerName: string;
  model: string;
  label: string;
  effort?: { id: string; options: { id: string; label: string }[] } | null;
}

export interface T3CodeNativeSettings {
  language: string;
  notificationMode: string;
  defaultModelSelection: {
    instanceId: string;
    model: string;
    options?: { id: string; value: string | boolean }[];
  } | null;
  models: T3CodeNativeModel[];
  clientConnected: boolean;
}

export interface T3CodeProviderSwitchView {
  workspace: T3CodeProviderWorkspaceView;
  appliedLive: boolean;
  restartRequired: boolean;
  message: string;
}

export function hasT3codeResolvedIdentity(account: T3CodeAccount): boolean {
  const email = account.email?.trim() || '';
  return (account.identity_verified_at ?? 0) > 0 &&
    /^[^\s@]+@[^\s@]+$/.test(email) &&
    !(email.startsWith('clerk-') && email.endsWith('@t3code.cloud'));
}

export function getT3codeAccountDisplayEmail(account: T3CodeAccount): string {
  return hasT3codeResolvedIdentity(account) ? account.email.trim() : '';
}

export function getT3codePlanBadge(account: T3CodeAccount, unlinkedLabel = 'UNKNOWN'): string {
  if ((account.relay_verified_at ?? 0) > 0) {
    const plan = account.plan_type?.trim();
    if (plan) return plan;
    if (account.t3chat_linked === false) return unlinkedLabel;
  }
  return 'UNKNOWN';
}

export function getT3codeUsage(account: T3CodeAccount): CodebuddyUsage {
  const isNormal = hasT3codeQuotaData(account) && !account.quota_query_last_error;
  return {
    dosageNotifyCode: isNormal ? '0' : 'UNAVAILABLE',
    dosageNotifyZh: account.quota_query_last_error || undefined,
    dosageNotifyEn: account.quota_query_last_error || undefined,
    isNormal,
    inlineSuggestionsUsedPercent: null,
    chatMessagesUsedPercent: null,
    allowanceResetAt: null,
  };
}

export function getT3codeQuotaUpdatedAtMs(account: T3CodeAccount): number | null {
  return hasVerifiedTunnelQuota(account) ? account.relay_verified_at * 1000 : null;
}

function isTunnelCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

function hasVerifiedTunnelQuota(account: T3CodeAccount): account is T3CodeAccount & {
  quota_used: number;
  quota_total: number;
  relay_verified_at: number;
} {
  return typeof account.relay_verified_at === 'number' && account.relay_verified_at > 0 &&
    isTunnelCount(account.quota_used) && isTunnelCount(account.quota_total);
}

export function hasT3codeQuotaData(account: T3CodeAccount): boolean {
  return hasVerifiedTunnelQuota(account);
}

export function getT3codeQuotaGroups(
  account: T3CodeAccount,
  t: (key: string, defaultValue?: string) => string,
): QuotaCategoryGroup[] {
  if (!hasVerifiedTunnelQuota(account)) return [];

  const { quota_used: used, quota_total: total } = account;
  const remain = Math.max(0, total - used);
  const usedPercent = total > 0 ? Math.min(100, (used / total) * 100) : 0;
  const remainPercent = total > 0 ? (remain / total) * 100 : null;
  const label = t('t3code.quota.included');
  const resource: OfficialQuotaResource = {
    packageCode: 't3code-tunnels',
    packageName: label,
    cycleStartTime: null,
    cycleEndTime: null,
    deductionEndTime: null,
    expiredTime: null,
    total,
    used,
    remain,
    usedPercent,
    remainPercent,
    refreshAt: null,
    expireAt: null,
    isBasePackage: true,
  };
  return [{
    key: 'base',
    label,
    quotaClass: 'base',
    items: [resource],
    visible: true,
    used,
    remain,
    total,
    usedPercent,
    remainPercent,
  }];
}
