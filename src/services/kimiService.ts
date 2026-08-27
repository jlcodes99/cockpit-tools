import { invoke } from '@tauri-apps/api/core';
import type { KimiAccount, KimiQuota, KimiUsageRow } from '../types/kimi';

export interface KimiOAuthLoginStartResponse {
  loginId: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
}

/** IPC wire for KimiAccountView (`#[serde(rename_all = "camelCase")]`). */
interface RawKimiUsageRow {
  name?: string | null;
  windowUnit?: string | null;
  windowDuration?: number | null;
  used: number;
  limit: number;
  resetAt?: string | null;
}

interface RawKimiQuota {
  weeklyUsed?: number | null;
  weeklyLimit?: number | null;
  weeklyResetAt?: string | null;
  limits?: RawKimiUsageRow[];
  boosterBalanceCents?: number | null;
  boosterTotalCents?: number | null;
  boosterCurrency?: string | null;
  userLevelName?: string | null;
  region?: string | null;
}

interface RawKimiAccount {
  id: string;
  email: string;
  accessToken?: string;
  tags?: string[] | null;
  nickname?: string | null;
  userId?: string | null;
  avatar?: string | null;
  expiresAt?: number | null;
  planType?: string;
  quota?: RawKimiQuota | null;
  status?: string | null;
  statusReason?: string | null;
  quotaQueryLastError?: string | null;
  quotaQueryLastErrorAt?: number | null;
  usageUpdatedAt?: number | null;
  createdAt: number;
  lastUsed: number;
}

function fromRawUsageRow(raw: RawKimiUsageRow): KimiUsageRow {
  return {
    name: raw.name,
    windowUnit: raw.windowUnit,
    windowDuration: raw.windowDuration,
    used: raw.used,
    limit: raw.limit,
    resetAt: raw.resetAt,
  };
}

function fromRawQuota(raw?: RawKimiQuota | null): KimiQuota | null {
  if (!raw) return null;
  return {
    weeklyUsed: raw.weeklyUsed,
    weeklyLimit: raw.weeklyLimit,
    weeklyResetAt: raw.weeklyResetAt,
    limits: (raw.limits ?? []).map(fromRawUsageRow),
    boosterBalanceCents: raw.boosterBalanceCents,
    boosterTotalCents: raw.boosterTotalCents,
    boosterCurrency: raw.boosterCurrency,
    userLevelName: raw.userLevelName,
    region: raw.region,
  };
}

/** Map IPC camelCase account view → app snake_case model. Token always forced empty. */
export function fromRawKimiAccount(raw: RawKimiAccount): KimiAccount {
  return {
    id: raw.id,
    email: raw.email,
    access_token: '',
    tags: raw.tags,
    nickname: raw.nickname,
    user_id: raw.userId,
    avatar: raw.avatar,
    expires_at: raw.expiresAt,
    plan_type: raw.planType,
    quota: fromRawQuota(raw.quota),
    status: raw.status,
    status_reason: raw.statusReason,
    quota_query_last_error: raw.quotaQueryLastError,
    quota_query_last_error_at: raw.quotaQueryLastErrorAt,
    usage_updated_at: raw.usageUpdatedAt,
    created_at: raw.createdAt,
    last_used: raw.lastUsed,
  };
}

function mapAccounts(raw: RawKimiAccount[] | null | undefined): KimiAccount[] {
  return (raw ?? []).map(fromRawKimiAccount);
}

export async function listKimiAccounts(): Promise<KimiAccount[]> {
  return mapAccounts(await invoke<RawKimiAccount[]>('list_kimi_accounts'));
}

export async function deleteKimiAccount(accountId: string): Promise<void> {
  await invoke('delete_kimi_account', { accountId });
}

export async function deleteKimiAccounts(accountIds: string[]): Promise<void> {
  await invoke('delete_kimi_accounts', { accountIds });
}

export async function importKimiFromJson(
  jsonContent: string,
): Promise<KimiAccount[]> {
  return mapAccounts(
    await invoke<RawKimiAccount[]>('import_kimi_from_json', { jsonContent }),
  );
}

export async function importKimiFromLocal(): Promise<KimiAccount[]> {
  return mapAccounts(await invoke<RawKimiAccount[]>('import_kimi_from_local'));
}

export async function exportKimiAccounts(
  accountIds: string[],
): Promise<string> {
  return await invoke('export_kimi_accounts', { accountIds });
}

export async function startKimiOAuthLogin(): Promise<KimiOAuthLoginStartResponse> {
  return await invoke('kimi_oauth_login_start');
}

export async function completeKimiOAuthLogin(
  loginId: string,
  reauthAccountId?: string | null,
): Promise<KimiAccount> {
  const raw = await invoke<RawKimiAccount>('kimi_oauth_login_complete', {
    loginId,
    reauthAccountId: reauthAccountId ?? null,
  });
  return fromRawKimiAccount(raw);
}

export async function cancelKimiOAuthLogin(loginId?: string): Promise<void> {
  await invoke('kimi_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function refreshKimiAccount(
  accountId: string,
): Promise<KimiAccount> {
  return fromRawKimiAccount(
    await invoke<RawKimiAccount>('refresh_kimi_account', { accountId }),
  );
}

export async function refreshAllKimiAccounts(): Promise<number> {
  return await invoke('refresh_all_kimi_accounts');
}

export async function switchKimiAccount(accountId: string): Promise<string> {
  return await invoke('switch_kimi_account', { accountId });
}

export async function updateKimiAccountTags(
  accountId: string,
  tags: string[],
): Promise<KimiAccount> {
  return fromRawKimiAccount(
    await invoke<RawKimiAccount>('update_kimi_account_tags', { accountId, tags }),
  );
}

export async function getKimiCurrentAccountId(): Promise<string | null> {
  return await invoke('get_kimi_current_account_id');
}

export async function getKimiAccountsIndexPath(): Promise<string> {
  return await invoke('get_kimi_accounts_index_path');
}

export interface KimiCliStatus {
  available: boolean;
  binaryPath?: string | null;
  configuredPath?: string | null;
  version?: string | null;
  source?: string | null;
  message?: string | null;
  checkedAt?: number;
  home: string;
  configuredHome?: string | null;
}

export async function getKimiCliStatus(): Promise<KimiCliStatus> {
  return await invoke('kimi_get_cli_status');
}

export async function updateKimiCliRuntimeConfig(
  kimiCliPath?: string | null,
): Promise<KimiCliStatus> {
  return await invoke('kimi_update_cli_runtime_config', {
    kimiCliPath: kimiCliPath?.trim() || null,
  });
}

export async function updateKimiHomeConfig(
  kimiCodeHome?: string | null,
): Promise<KimiCliStatus> {
  return await invoke('kimi_update_home_config', {
    kimiCodeHome: kimiCodeHome?.trim() || null,
  });
}
