import { invoke } from '@tauri-apps/api/core';
import { QoderCnAccount } from '../types/qoderCn';

export interface QoderCnOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

type QoderCnOAuthStartResponseRaw = Partial<QoderCnOAuthStartResponse> & {
  login_id?: string;
  verification_uri?: string;
  expires_in?: number;
  interval_seconds?: number;
  callback_url?: string | null;
};

function normalizeQoderCnOAuthStartResponse(raw: QoderCnOAuthStartResponseRaw): QoderCnOAuthStartResponse {
  const loginId = raw.loginId ?? raw.login_id ?? '';
  const verificationUri = raw.verificationUri ?? raw.verification_uri ?? '';
  const expiresIn = Number(raw.expiresIn ?? raw.expires_in ?? 0);
  const intervalSeconds = Number(raw.intervalSeconds ?? raw.interval_seconds ?? 0);
  const callbackUrl = raw.callbackUrl ?? raw.callback_url ?? null;

  if (!loginId || !verificationUri) {
    throw new Error('Qoder CN OAuth start 响应缺少关键字段');
  }

  return {
    loginId,
    verificationUri,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : 600,
    intervalSeconds: Number.isFinite(intervalSeconds) && intervalSeconds > 0 ? intervalSeconds : 1,
    callbackUrl,
  };
}

export async function listQoderCnAccounts(): Promise<QoderCnAccount[]> {
  return await invoke('list_qoder_cn_accounts');
}

export async function deleteQoderCnAccount(accountId: string): Promise<void> {
  return await invoke('delete_qoder_cn_account', { accountId });
}

export async function deleteQoderCnAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_qoder_cn_accounts', { accountIds });
}

export async function importQoderCnFromJson(jsonContent: string): Promise<QoderCnAccount[]> {
  return await invoke('import_qoder_cn_from_json', { jsonContent });
}

export async function importQoderCnFromLocal(): Promise<QoderCnAccount[]> {
  return await invoke('import_qoder_cn_from_local');
}

export async function qoderCnOauthLoginStart(): Promise<QoderCnOAuthStartResponse> {
  const raw = await invoke<QoderCnOAuthStartResponseRaw>('qoder_cn_oauth_login_start');
  return normalizeQoderCnOAuthStartResponse(raw);
}

export async function qoderCnOauthLoginPeek(): Promise<QoderCnOAuthStartResponse | null> {
  const raw = await invoke<QoderCnOAuthStartResponseRaw | null>('qoder_cn_oauth_login_peek');
  if (!raw) return null;
  try {
    return normalizeQoderCnOAuthStartResponse(raw);
  } catch {
    return null;
  }
}

export async function qoderCnOauthLoginComplete(loginId: string): Promise<QoderCnAccount> {
  return await invoke('qoder_cn_oauth_login_complete', { loginId });
}

export async function qoderCnOauthLoginCancel(loginId?: string): Promise<void> {
  return await invoke('qoder_cn_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function exportQoderCnAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_qoder_cn_accounts', { accountIds });
}

export async function refreshQoderCnToken(accountId: string): Promise<QoderCnAccount> {
  return await invoke('refresh_qoder_cn_token', { accountId });
}

export async function refreshAllQoderCnTokens(): Promise<number> {
  return await invoke('refresh_all_qoder_cn_tokens');
}

export async function switchQoderCnAccount(accountId: string): Promise<string> {
  return await invoke('switch_qoder_cn_account', { accountId });
}

export async function addQoderCnAccountWithToken(token: string): Promise<QoderCnAccount> {
  return await invoke('add_qoder_cn_account_with_token', { token });
}

export async function updateQoderCnAccountTags(
  accountId: string,
  tags: string[],
): Promise<QoderCnAccount> {
  return await invoke('update_qoder_cn_account_tags', { accountId, tags });
}

export async function getQoderCnAccountsIndexPath(): Promise<string> {
  return await invoke('get_qoder_cn_accounts_index_path');
}
