import { invoke } from '@tauri-apps/api/core';
import { TraeAccount } from '../types/trae';
import { getProviderCurrentAccountId } from './providerCurrentAccountService';

export type TraePlatformId = 'trae' | 'trae_solo' | 'trae_cn' | 'trae_solo_cn';

export interface TraeOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

type TraeOAuthStartResponseRaw = Partial<TraeOAuthStartResponse> & {
  login_id?: string;
  verification_uri?: string;
  expires_in?: number;
  interval_seconds?: number;
  callback_url?: string | null;
};

function normalizeTraeOAuthStartResponse(raw: TraeOAuthStartResponseRaw): TraeOAuthStartResponse {
  const loginId = raw.loginId ?? raw.login_id ?? '';
  const verificationUri = raw.verificationUri ?? raw.verification_uri ?? '';
  const expiresIn = Number(raw.expiresIn ?? raw.expires_in ?? 0);
  const intervalSeconds = Number(raw.intervalSeconds ?? raw.interval_seconds ?? 0);
  const callbackUrl = raw.callbackUrl ?? raw.callback_url ?? null;

  if (!loginId || !verificationUri) {
    throw new Error('Trae OAuth start 响应缺少关键字段');
  }

  return {
    loginId,
    verificationUri,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : 600,
    intervalSeconds: Number.isFinite(intervalSeconds) && intervalSeconds > 0 ? intervalSeconds : 1,
    callbackUrl,
  };
}

export async function listTraeAccounts(): Promise<TraeAccount[]> {
  return await invoke('list_trae_accounts');
}

export async function deleteTraeAccount(accountId: string): Promise<void> {
  return await invoke('delete_trae_account', { accountId });
}

export async function deleteTraeAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_trae_accounts', { accountIds });
}

export async function importTraeFromJson(jsonContent: string): Promise<TraeAccount[]> {
  return await invoke('import_trae_from_json', { jsonContent });
}

export async function importTraeFromLocal(
  platformId: TraePlatformId = 'trae',
): Promise<TraeAccount[]> {
  return await invoke('import_trae_from_local', { platformId });
}

export async function traeOauthLoginStart(
  platformId: TraePlatformId = 'trae',
): Promise<TraeOAuthStartResponse> {
  const raw = await invoke<TraeOAuthStartResponseRaw>('trae_oauth_login_start', { platformId });
  return normalizeTraeOAuthStartResponse(raw);
}

export async function traeOauthLoginComplete(
  loginId: string,
  platformId: TraePlatformId = 'trae',
): Promise<TraeAccount> {
  return await invoke('trae_oauth_login_complete', { loginId, platformId });
}

export async function traeOauthLoginCancel(
  loginId?: string,
  platformId: TraePlatformId = 'trae',
): Promise<void> {
  return await invoke('trae_oauth_login_cancel', { loginId: loginId ?? null, platformId });
}

export async function traeOauthSubmitCallbackUrl(
  loginId: string,
  callbackUrl: string,
  platformId: TraePlatformId = 'trae',
): Promise<void> {
  return await invoke('trae_oauth_submit_callback_url', { loginId, callbackUrl, platformId });
}

export async function exportTraeAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_trae_accounts', { accountIds });
}

export async function refreshTraeToken(accountId: string): Promise<TraeAccount> {
  return await invoke('refresh_trae_token', { accountId });
}

export async function refreshAllTraeTokens(platformId?: TraePlatformId): Promise<number> {
  if (platformId) {
    return await invoke('refresh_trae_tokens_for_platform', { platformId });
  }
  return await invoke('refresh_all_trae_tokens');
}

export async function addTraeAccountWithToken(accessToken: string): Promise<TraeAccount> {
  return await invoke('add_trae_account_with_token', { accessToken });
}

export async function updateTraeAccountTags(accountId: string, tags: string[]): Promise<TraeAccount> {
  return await invoke('update_trae_account_tags', { accountId, tags });
}

export async function getTraeAccountsIndexPath(): Promise<string> {
  return await invoke('get_trae_accounts_index_path');
}

export async function getTraeCurrentAccountId(
  platformId: TraePlatformId = 'trae',
): Promise<string | null> {
  return await getProviderCurrentAccountId(platformId);
}

export async function injectTraeAccount(
  accountId: string,
  platformId: TraePlatformId = 'trae',
): Promise<string> {
  return await invoke('inject_trae_account', { accountId, platformId });
}

// ============ 签到功能 ============

/** 签到状态响应（与 Rust 后端 CheckinStatusResult 对齐） */
export interface TraeCheckinStatusResult {
  checked_in: boolean;
  consecutive_days: number;
  total_credits: number;
  credits_earned_today: number;
  checkin_date: string;
  message: string;
}

const TRAE_CHECKIN_DEVICE_ID_KEY = 'agtools.trae.checkin_device_id';

/** 获取或生成 Trae 签到使用的设备 ID */
export function getTraeCheckinDeviceId(): string {
  if (typeof window === 'undefined') return '';
  try {
    let deviceId = localStorage.getItem(TRAE_CHECKIN_DEVICE_ID_KEY);
    if (!deviceId) {
      deviceId = `${Date.now()}_${Math.random().toString(36).substring(2, 11)}`;
      localStorage.setItem(TRAE_CHECKIN_DEVICE_ID_KEY, deviceId);
    }
    return deviceId;
  } catch {
    return '';
  }
}

/** 获取 Trae 账号的今日签到状态 */
export async function getTraeCheckinStatus(
  accountId: string,
): Promise<TraeCheckinStatusResult> {
  const deviceId = getTraeCheckinDeviceId();
  return await invoke('get_trae_checkin_status', { accountId, deviceId });
}

/** 领取 Trae 账号的今日签到积分 */
export async function claimTraeCheckin(
  accountId: string,
): Promise<TraeCheckinStatusResult> {
  const deviceId = getTraeCheckinDeviceId();
  return await invoke('claim_trae_checkin', { accountId, deviceId });
}
