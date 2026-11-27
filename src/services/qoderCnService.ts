import { invoke } from '@tauri-apps/api/core';
import { QoderAccount } from '../types/qoder';
import type { QoderSwitchOutcome } from '../types/qoder';
import type { QoderRegion, QoderClient } from '../utils/qoderRegion';

export interface QoderOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

type QoderOAuthStartResponseRaw = Partial<QoderOAuthStartResponse> & {
  login_id?: string;
  verification_uri?: string;
  expires_in?: number;
  interval_seconds?: number;
  callback_url?: string | null;
};

function normalizeQoderOAuthStartResponse(raw: QoderOAuthStartResponseRaw): QoderOAuthStartResponse {
  const loginId = raw.loginId ?? raw.login_id ?? '';
  const verificationUri = raw.verificationUri ?? raw.verification_uri ?? '';
  const expiresIn = Number(raw.expiresIn ?? raw.expires_in ?? 0);
  const intervalSeconds = Number(raw.intervalSeconds ?? raw.interval_seconds ?? 0);
  const callbackUrl = raw.callbackUrl ?? raw.callback_url ?? null;

  if (!loginId || !verificationUri) {
    throw new Error('Qoder OAuth start 响应缺少关键字段');
  }

  return {
    loginId,
    verificationUri,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : 600,
    intervalSeconds: Number.isFinite(intervalSeconds) && intervalSeconds > 0 ? intervalSeconds : 1,
    callbackUrl,
  };
}

export async function listQoderAccounts(): Promise<QoderAccount[]> {
  return await invoke('list_qoder_cn_accounts');
}

export async function deleteQoderAccount(accountId: string): Promise<void> {
  return await invoke('delete_qoder_cn_account', { accountId });
}

export async function deleteQoderAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_qoder_cn_accounts', { accountIds });
}

export async function importQoderFromJson(jsonContent: string, region?: QoderRegion): Promise<QoderAccount[]> {
  return await invoke('import_qoder_cn_from_json', { jsonContent, region });
}

export async function importQoderFromLocal(): Promise<QoderAccount[]> {
  return await invoke('import_qoder_cn_from_local');
}

export async function qoderOauthLoginStart(region: QoderRegion = 'cn'): Promise<QoderOAuthStartResponse> {
  const raw = await invoke<QoderOAuthStartResponseRaw>('qoder_cn_oauth_login_start', { region });
  return normalizeQoderOAuthStartResponse(raw);
}

export async function importQoderNative(region: QoderRegion, client: QoderClient): Promise<QoderAccount[]> {
  return await invoke('import_qoder_cn_native', { region, client });
}
export async function injectQoderNative(
  accountId: string,
  client: QoderClient,
  options?: { launch: boolean; workingDir?: string; terminal?: string },
): Promise<QoderSwitchOutcome> {
  return await invoke('inject_qoder_cn_native', { accountId, client, ...options });
}

export async function qoderOauthLoginPeek(): Promise<QoderOAuthStartResponse | null> {
  const raw = await invoke<QoderOAuthStartResponseRaw | null>('qoder_cn_oauth_login_peek');
  if (!raw) return null;
  try {
    return normalizeQoderOAuthStartResponse(raw);
  } catch {
    return null;
  }
}

export async function qoderOauthLoginComplete(loginId: string): Promise<QoderAccount> {
  return await invoke('qoder_cn_oauth_login_complete', { loginId });
}

export async function qoderOauthLoginCancel(loginId?: string): Promise<void> {
  return await invoke('qoder_cn_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function exportQoderAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_qoder_cn_accounts', { accountIds });
}

export async function refreshQoderToken(accountId: string): Promise<QoderAccount> {
  return await invoke('refresh_qoder_cn_token', { accountId });
}

export interface QoderCheckinStatus {
  checkedIn: boolean;
  active: boolean;
  amount?: number | null;
  validityDays?: number | null;
  nextRefreshAt?: number | null;
  campaignKey?: string | null;
  message: string;
}

export async function getQoderCnCheckinStatus(accountId: string): Promise<QoderCheckinStatus> {
  return await invoke('get_qoder_cn_checkin_status', { accountId });
}

export async function claimQoderCnCheckin(accountId: string): Promise<QoderCheckinStatus> {
  return await invoke('claim_qoder_cn_checkin', { accountId });
}

export async function refreshAllQoderTokens(): Promise<number> {
  return await invoke('refresh_all_qoder_cn_tokens');
}

export async function injectQoderAccount(accountId: string): Promise<string> {
  return await invoke('inject_qoder_cn_account', { accountId });
}

export async function updateQoderAccountTags(
  accountId: string,
  tags: string[],
): Promise<QoderAccount> {
  return await invoke('update_qoder_cn_account_tags', { accountId, tags });
}

export async function getQoderAccountsIndexPath(): Promise<string> {
  return await invoke('get_qoder_cn_accounts_index_path');
}
