import { invoke } from '@tauri-apps/api/core';
import type { WorkbuddyAccount } from '../types/workbuddy';
import type { CheckinResponse, CheckinStatusResponse } from '../types/codebuddy';

export interface WorkbuddyOAuthLoginStartResponse {
  loginId: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
}

export async function listWorkbuddyAccounts(): Promise<WorkbuddyAccount[]> {
  return await invoke('list_workbuddy_accounts');
}

export async function deleteWorkbuddyAccount(accountId: string): Promise<void> {
  return await invoke('delete_workbuddy_account', { accountId });
}

export async function deleteWorkbuddyAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_workbuddy_accounts', { accountIds });
}

export async function importWorkbuddyFromJson(jsonContent: string): Promise<WorkbuddyAccount[]> {
  return await invoke('import_workbuddy_from_json', { jsonContent });
}

export async function importWorkbuddyFromLocal(): Promise<WorkbuddyAccount[]> {
  return await invoke('import_workbuddy_from_local');
}

export async function exportWorkbuddyAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_workbuddy_accounts', { accountIds });
}

export async function refreshWorkbuddyToken(accountId: string): Promise<WorkbuddyAccount> {
  return await invoke('refresh_workbuddy_token', { accountId });
}

export async function refreshAllWorkbuddyTokens(): Promise<number> {
  return await invoke('refresh_all_workbuddy_tokens');
}

export async function startWorkbuddyOAuthLogin(): Promise<WorkbuddyOAuthLoginStartResponse> {
  return await invoke('workbuddy_oauth_login_start');
}

export async function completeWorkbuddyOAuthLogin(loginId: string): Promise<WorkbuddyAccount> {
  return await invoke('workbuddy_oauth_login_complete', { loginId });
}

export async function cancelWorkbuddyOAuthLogin(loginId?: string): Promise<void> {
  return await invoke('workbuddy_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function addWorkbuddyAccountWithToken(accessToken: string): Promise<WorkbuddyAccount> {
  return await invoke('add_workbuddy_account_with_token', { accessToken });
}

export async function updateWorkbuddyAccountTags(accountId: string, tags: string[]): Promise<WorkbuddyAccount> {
  return await invoke('update_workbuddy_account_tags', { accountId, tags });
}

export async function getWorkbuddyAccountsIndexPath(): Promise<string> {
  return await invoke('get_workbuddy_accounts_index_path');
}

export async function injectWorkbuddyToVSCode(accountId: string): Promise<string> {
  return await invoke('inject_workbuddy_to_vscode', { accountId });
}

export async function checkinWorkbuddy(accountId: string): Promise<CheckinResponse> {
  return await invoke('checkin_workbuddy', { accountId });
}

export async function getCheckinStatusWorkbuddy(accountId: string): Promise<CheckinStatusResponse> {
  return await invoke('get_checkin_status_workbuddy', { accountId });
}
