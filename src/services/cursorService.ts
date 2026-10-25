import { invoke } from '@tauri-apps/api/core';
import { CursorAccount } from '../types/cursor';

export interface CursorOAuthLoginStartResponse {
  loginId: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
}

/** Cursor OAuth: 开始登录（生成 PKCE，返回浏览器 URL） */
export async function startCursorOAuthLogin(): Promise<CursorOAuthLoginStartResponse> {
  return await invoke('cursor_oauth_login_start');
}

/** Cursor OAuth: 等待轮询完成（用户在浏览器完成登录后返回账号） */
export async function completeCursorOAuthLogin(loginId: string): Promise<CursorAccount> {
  return await invoke('cursor_oauth_login_complete', { loginId });
}

/** Cursor OAuth: 取消登录 */
export async function cancelCursorOAuthLogin(loginId?: string): Promise<void> {
  return await invoke('cursor_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function listCursorAccounts(): Promise<CursorAccount[]> {
  return await invoke('list_cursor_accounts');
}

export async function deleteCursorAccount(accountId: string): Promise<void> {
  return await invoke('delete_cursor_account', { accountId });
}

export async function deleteCursorAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_cursor_accounts', { accountIds });
}

export async function importCursorFromJson(jsonContent: string): Promise<CursorAccount[]> {
  return await invoke('import_cursor_from_json', { jsonContent });
}

export async function importCursorFromLocal(): Promise<CursorAccount[]> {
  return await invoke('import_cursor_from_local');
}

export async function exportCursorAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_cursor_accounts', { accountIds });
}

export async function refreshCursorToken(accountId: string): Promise<CursorAccount> {
  return await invoke('refresh_cursor_token', { accountId });
}

export async function refreshAllCursorTokens(): Promise<number> {
  return await invoke('refresh_all_cursor_tokens');
}

export async function addCursorAccountWithToken(accessToken: string): Promise<CursorAccount> {
  return await invoke('add_cursor_account_with_token', { accessToken });
}

export async function updateCursorAccountTags(accountId: string, tags: string[]): Promise<CursorAccount> {
  return await invoke('update_cursor_account_tags', { accountId, tags });
}

export async function setCursorManualQuotaExhausted(
  accountId: string,
  exhausted: boolean,
): Promise<CursorAccount> {
  return await invoke('set_cursor_manual_quota_exhausted', { accountId, exhausted });
}

export async function getCursorAccountsIndexPath(): Promise<string> {
  return await invoke('get_cursor_accounts_index_path');
}

export async function injectCursorAccount(accountId: string): Promise<string> {
  return await invoke('inject_cursor_account', { accountId });
}
