import { invoke } from '@tauri-apps/api/core';
import { QoderAccount } from '../types/qoder';
import {
  QoderOAuthStartResponse,
  QoderOAuthStartResponseRaw,
  normalizeQoderOAuthStartResponse,
} from './qoderService';

export async function listQoderCnAccounts(): Promise<QoderAccount[]> {
  return await invoke('list_qoder_cn_accounts');
}

export async function deleteQoderCnAccount(accountId: string): Promise<void> {
  return await invoke('delete_qoder_cn_account', { accountId });
}

export async function deleteQoderCnAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_qoder_cn_accounts', { accountIds });
}

export async function updateQoderCnAccountTags(
  accountId: string,
  tags: string[],
): Promise<QoderAccount> {
  return await invoke('update_qoder_cn_account_tags', { accountId, tags });
}

export async function getQoderCnAccountsIndexPath(): Promise<string> {
  return await invoke('get_qoder_cn_accounts_index_path');
}

export async function qoderCnOauthLoginStart(): Promise<QoderOAuthStartResponse> {
  const raw = await invoke<QoderOAuthStartResponseRaw>('qoder_cn_oauth_login_start');
  return normalizeQoderOAuthStartResponse(raw);
}

export async function qoderCnOauthLoginPeek(): Promise<QoderOAuthStartResponse | null> {
  const raw = await invoke<QoderOAuthStartResponseRaw | null>('qoder_cn_oauth_login_peek');
  if (!raw) return null;
  try {
    return normalizeQoderOAuthStartResponse(raw);
  } catch {
    return null;
  }
}

export async function qoderCnOauthLoginComplete(loginId: string): Promise<QoderAccount> {
  return await invoke('qoder_cn_oauth_login_complete', { loginId });
}

export async function qoderCnOauthLoginCancel(loginId?: string): Promise<void> {
  return await invoke('qoder_cn_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function refreshQoderCnToken(accountId: string): Promise<QoderAccount> {
  return await invoke('refresh_qoder_cn_token', { accountId });
}

export async function refreshAllQoderCnTokens(): Promise<number> {
  return await invoke('refresh_all_qoder_cn_tokens');
}
