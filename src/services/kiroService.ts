import { invoke } from '@tauri-apps/api/core';
import { KiroAccount } from '../types/kiro';
import i18n from '../i18n';
import { formatKiroImportError } from './kiroImportErrors';

export interface KiroOAuthLoginStartResponse {
  loginId: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

/** 列出所有 Kiro 账号 */
export async function listKiroAccounts(): Promise<KiroAccount[]> {
  return await invoke('list_kiro_accounts');
}

/** 删除 Kiro 账号 */
export async function deleteKiroAccount(accountId: string): Promise<void> {
  return await invoke('delete_kiro_account', { accountId });
}

/** 批量删除 Kiro 账号 */
export async function deleteKiroAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_kiro_accounts', { accountIds });
}

/** 从 JSON 字符串导入账号 */
export async function importKiroFromJson(jsonContent: string): Promise<KiroAccount[]> {
  try {
    return await invoke('import_kiro_from_json', { jsonContent });
  } catch (error) {
    throw new Error(formatKiroImportError(error instanceof Error ? error.message : String(error), i18n.t.bind(i18n)));
  }
}

/** 从本机 Kiro 客户端导入当前登录账号 */
export async function importKiroFromLocal(): Promise<KiroAccount[]> {
  return await invoke('import_kiro_from_local');
}

/** 导出 Kiro 账号 */
export async function exportKiroAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_kiro_accounts', { accountIds });
}

/** 刷新单个账号 token/usage */
export async function refreshKiroToken(accountId: string): Promise<KiroAccount> {
  try {
    return await invoke('refresh_kiro_token', { accountId });
  } catch (error) {
    throw new Error(formatKiroImportError(error instanceof Error ? error.message : String(error), i18n.t.bind(i18n)));
  }
}

/** 刷新全部账号 token/usage */
export async function refreshAllKiroTokens(): Promise<number> {
  return await invoke('refresh_all_kiro_tokens');
}

/** Kiro OAuth：开始登录（浏览器授权 + 本地回调） */
export type KiroOAuthLoginStartOptions = {
  method: 'social' | 'builderId' | 'enterprise';
  region?: string;
  startUrl?: string;
};

export async function startKiroOAuthLogin(
  options?: KiroOAuthLoginStartOptions,
): Promise<KiroOAuthLoginStartResponse> {
  return await invoke('kiro_oauth_login_start', { options: options ?? null });
}

/** Kiro OAuth：完成登录（等待本地回调，直到成功/失败/超时） */
export async function completeKiroOAuthLogin(loginId: string): Promise<KiroAccount> {
  return await invoke('kiro_oauth_login_complete', { loginId });
}

/** Kiro OAuth：取消登录 */
export async function cancelKiroOAuthLogin(loginId?: string): Promise<void> {
  return await invoke('kiro_oauth_login_cancel', { loginId: loginId ?? null });
}

/** Kiro OAuth：手动提交回调链接 */
export async function submitKiroOAuthCallbackUrl(
  loginId: string,
  callbackUrl: string,
): Promise<void> {
  return await invoke('kiro_oauth_submit_callback_url', { loginId, callbackUrl });
}

/** 通过 Kiro access token 添加账号 */
export async function addKiroAccountWithToken(accessToken: string): Promise<KiroAccount> {
  return await invoke('add_kiro_account_with_token', {
    accessToken,
    access_token: accessToken,
  });
}

export async function updateKiroAccountTags(accountId: string, tags: string[]): Promise<KiroAccount> {
  return await invoke('update_kiro_account_tags', { accountId, tags });
}

export async function getKiroAccountsIndexPath(): Promise<string> {
  return await invoke('get_kiro_accounts_index_path');
}

/** 将 Kiro 账号注入到 Kiro 默认实例 */
export async function injectKiroToVSCode(accountId: string): Promise<string> {
  return await invoke('inject_kiro_to_vscode', { accountId });
}
