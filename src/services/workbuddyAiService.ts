import { invoke } from '@tauri-apps/api/core';
import type { WorkbuddyAccount } from '../types/workbuddy';
import type { WorkbuddyOAuthLoginStartResponse } from './workbuddyService';

export async function listWorkbuddyAiAccounts(): Promise<WorkbuddyAccount[]> {
  return await invoke('list_workbuddy_ai_accounts');
}

export async function deleteWorkbuddyAiAccount(accountId: string): Promise<void> {
  return await invoke('delete_workbuddy_ai_account', { accountId });
}

export async function deleteWorkbuddyAiAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_workbuddy_ai_accounts', { accountIds });
}

export async function importWorkbuddyAiFromJson(jsonContent: string): Promise<WorkbuddyAccount[]> {
  return await invoke('import_workbuddy_ai_from_json', { jsonContent });
}

export async function importWorkbuddyAiFromLocal(): Promise<WorkbuddyAccount[]> {
  return await invoke('import_workbuddy_ai_from_local');
}

export async function exportWorkbuddyAiAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_workbuddy_ai_accounts', { accountIds });
}

export async function refreshWorkbuddyAiToken(accountId: string): Promise<WorkbuddyAccount> {
  return await invoke('refresh_workbuddy_ai_token', { accountId });
}

export async function refreshAllWorkbuddyAiTokens(): Promise<number> {
  return await invoke('refresh_all_workbuddy_ai_tokens');
}

export async function startWorkbuddyAiOAuthLogin(): Promise<WorkbuddyOAuthLoginStartResponse> {
  return await invoke('workbuddy_ai_oauth_login_start');
}

export async function completeWorkbuddyAiOAuthLogin(loginId: string): Promise<WorkbuddyAccount> {
  return await invoke('workbuddy_ai_oauth_login_complete', { loginId });
}

export async function cancelWorkbuddyAiOAuthLogin(loginId?: string): Promise<void> {
  return await invoke('workbuddy_ai_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function addWorkbuddyAiAccountWithToken(accessToken: string): Promise<WorkbuddyAccount> {
  return await invoke('add_workbuddy_ai_account_with_token', { accessToken });
}

export async function updateWorkbuddyAiAccountTags(accountId: string, tags: string[]): Promise<WorkbuddyAccount> {
  return await invoke('update_workbuddy_ai_account_tags', { accountId, tags });
}

export async function injectWorkbuddyAiToVSCode(accountId: string): Promise<string> {
  return await invoke('inject_workbuddy_ai_to_vscode', { accountId });
}
