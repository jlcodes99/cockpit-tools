import { invoke } from '@tauri-apps/api/core';
import { QoderworkCnAccount } from '../types/qoderworkCn';

export interface QoderworkCnOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

type QoderworkCnOAuthStartResponseRaw = Partial<QoderworkCnOAuthStartResponse> & {
  login_id?: string;
  verification_uri?: string;
  expires_in?: number;
  interval_seconds?: number;
  callback_url?: string | null;
};

function normalizeQoderworkCnOAuthStartResponse(raw: QoderworkCnOAuthStartResponseRaw): QoderworkCnOAuthStartResponse {
  const loginId = raw.loginId ?? raw.login_id ?? '';
  const verificationUri = raw.verificationUri ?? raw.verification_uri ?? '';
  const expiresIn = Number(raw.expiresIn ?? raw.expires_in ?? 0);
  const intervalSeconds = Number(raw.intervalSeconds ?? raw.interval_seconds ?? 0);
  const callbackUrl = raw.callbackUrl ?? raw.callback_url ?? null;

  if (!loginId || !verificationUri) {
    throw new Error('QoderWork CN OAuth start 响应缺少关键字段');
  }

  return {
    loginId,
    verificationUri,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : 600,
    intervalSeconds: Number.isFinite(intervalSeconds) && intervalSeconds > 0 ? intervalSeconds : 1,
    callbackUrl,
  };
}

export async function listQoderworkCnAccounts(): Promise<QoderworkCnAccount[]> {
  return await invoke('list_qoderwork_cn_accounts');
}

export async function deleteQoderworkCnAccount(accountId: string): Promise<void> {
  return await invoke('delete_qoderwork_cn_account', { accountId });
}

export async function deleteQoderworkCnAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_qoderwork_cn_accounts', { accountIds });
}

export async function importQoderworkCnFromJson(jsonContent: string): Promise<QoderworkCnAccount[]> {
  return await invoke('import_qoderwork_cn_from_json', { jsonContent });
}

export async function importQoderworkCnFromLocal(): Promise<QoderworkCnAccount[]> {
  return await invoke('import_qoderwork_cn_from_local');
}

export async function qoderworkCnOauthLoginStart(): Promise<QoderworkCnOAuthStartResponse> {
  const raw = await invoke<QoderworkCnOAuthStartResponseRaw>('qoderwork_cn_oauth_login_start');
  return normalizeQoderworkCnOAuthStartResponse(raw);
}

export async function qoderworkCnOauthLoginPeek(): Promise<QoderworkCnOAuthStartResponse | null> {
  const raw = await invoke<QoderworkCnOAuthStartResponseRaw | null>('qoderwork_cn_oauth_login_peek');
  if (!raw) return null;
  try {
    return normalizeQoderworkCnOAuthStartResponse(raw);
  } catch {
    return null;
  }
}

export async function qoderworkCnOauthLoginComplete(loginId: string): Promise<QoderworkCnAccount> {
  return await invoke('qoderwork_cn_oauth_login_complete', { loginId });
}

export async function qoderworkCnOauthLoginCancel(loginId?: string): Promise<void> {
  return await invoke('qoderwork_cn_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function exportQoderworkCnAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_qoderwork_cn_accounts', { accountIds });
}

export async function refreshQoderworkCnToken(accountId: string): Promise<QoderworkCnAccount> {
  return await invoke('refresh_qoderwork_cn_token', { accountId });
}

export async function refreshAllQoderworkCnTokens(): Promise<number> {
  return await invoke('refresh_all_qoderwork_cn_tokens');
}

export async function switchQoderworkCnAccount(accountId: string): Promise<string> {
  return await invoke('switch_qoderwork_cn_account', { accountId });
}

export async function addQoderworkCnAccountWithToken(token: string): Promise<QoderworkCnAccount> {
  return await invoke('add_qoderwork_cn_account_with_token', { token });
}

export async function updateQoderworkCnAccountTags(
  accountId: string,
  tags: string[],
): Promise<QoderworkCnAccount> {
  return await invoke('update_qoderwork_cn_account_tags', { accountId, tags });
}

export async function getQoderworkCnAccountsIndexPath(): Promise<string> {
  return await invoke('get_qoderwork_cn_accounts_index_path');
}
