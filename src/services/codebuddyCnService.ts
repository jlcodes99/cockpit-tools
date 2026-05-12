import { invoke } from '@tauri-apps/api/core';
import type { CodebuddyAccount, CheckinStatusResponse, CheckinResponse } from '../types/codebuddy';

export interface CodebuddyCnOAuthLoginStartResponse {
  loginId: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
}

export async function listCodebuddyCnAccounts(): Promise<CodebuddyAccount[]> {
  return await invoke('list_codebuddy_cn_accounts');
}

export async function deleteCodebuddyCnAccount(accountId: string): Promise<void> {
  return await invoke('delete_codebuddy_cn_account', { accountId });
}

export async function deleteCodebuddyCnAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_codebuddy_cn_accounts', { accountIds });
}

export async function importCodebuddyCnFromJson(jsonContent: string): Promise<CodebuddyAccount[]> {
  return await invoke('import_codebuddy_cn_from_json', { jsonContent });
}

export async function importCodebuddyCnFromLocal(): Promise<CodebuddyAccount[]> {
  return await invoke('import_codebuddy_cn_from_local');
}

export async function exportCodebuddyCnAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_codebuddy_cn_accounts', { accountIds });
}

export async function refreshCodebuddyCnToken(accountId: string): Promise<CodebuddyAccount> {
  return await invoke('refresh_codebuddy_cn_token', { accountId });
}

export async function refreshAllCodebuddyCnTokens(): Promise<number> {
  return await invoke('refresh_all_codebuddy_cn_tokens');
}

export async function startCodebuddyCnOAuthLogin(): Promise<CodebuddyCnOAuthLoginStartResponse> {
  return await invoke('codebuddy_cn_oauth_login_start');
}

export async function completeCodebuddyCnOAuthLogin(loginId: string): Promise<CodebuddyAccount> {
  return await invoke('codebuddy_cn_oauth_login_complete', { loginId });
}

export async function cancelCodebuddyCnOAuthLogin(loginId?: string): Promise<void> {
  return await invoke('codebuddy_cn_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function addCodebuddyCnAccountWithToken(accessToken: string): Promise<CodebuddyAccount> {
  return await invoke('add_codebuddy_cn_account_with_token', { accessToken });
}

export async function updateCodebuddyCnAccountTags(accountId: string, tags: string[]): Promise<CodebuddyAccount> {
  return await invoke('update_codebuddy_cn_account_tags', { accountId, tags });
}

export async function getCodebuddyCnAccountsIndexPath(): Promise<string> {
  return await invoke('get_codebuddy_cn_accounts_index_path');
}

export async function injectCodebuddyCnToVSCode(accountId: string): Promise<string> {
  return await invoke('inject_codebuddy_cn_to_vscode', { accountId });
}

export async function syncCodebuddyCnToWorkbuddy(): Promise<number> {
  return await invoke('sync_codebuddy_cn_to_workbuddy');
}

export async function syncWorkbuddyToCodebuddyCn(): Promise<number> {
  return await invoke('sync_workbuddy_to_codebuddy_cn');
}

// ==================== WorkBuddy 签到 API 函数 ====================

/**
 * 执行 WorkBuddy 签到
 * @param accountId 账号 ID
 * @returns 签到结果
 */
export async function checkinWorkbuddy(accountId: string): Promise<CheckinResponse> {
  return await invoke('checkin_workbuddy', { accountId });
}

/**
 * 获取 WorkBuddy 签到状态
 * @param accountId 账号 ID
 * @returns 签到状态信息
 */
export async function getCheckinStatusWorkbuddy(accountId: string): Promise<CheckinStatusResponse> {
  return await invoke('get_checkin_status_workbuddy', { accountId });
}
