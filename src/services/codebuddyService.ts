import { invoke } from '@tauri-apps/api/core';
import { CodebuddyAccount } from '../types/codebuddy';

export interface CodebuddyOAuthLoginStartResponse {
  loginId: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
}

export async function listCodebuddyAccounts(): Promise<CodebuddyAccount[]> {
  return await invoke('list_codebuddy_accounts');
}

export async function deleteCodebuddyAccount(accountId: string): Promise<void> {
  return await invoke('delete_codebuddy_account', { accountId });
}

export async function deleteCodebuddyAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_codebuddy_accounts', { accountIds });
}

export async function importCodebuddyFromJson(jsonContent: string): Promise<CodebuddyAccount[]> {
  return await invoke('import_codebuddy_from_json', { jsonContent });
}

export async function importCodebuddyFromLocal(): Promise<CodebuddyAccount[]> {
  return await invoke('import_codebuddy_from_local');
}

export async function exportCodebuddyAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_codebuddy_accounts', { accountIds });
}

export async function refreshCodebuddyToken(accountId: string): Promise<CodebuddyAccount> {
  return await invoke('refresh_codebuddy_token', { accountId });
}

export async function refreshAllCodebuddyTokens(): Promise<number> {
  return await invoke('refresh_all_codebuddy_tokens');
}

export async function startCodebuddyOAuthLogin(): Promise<CodebuddyOAuthLoginStartResponse> {
  return await invoke('codebuddy_oauth_login_start');
}

export async function completeCodebuddyOAuthLogin(loginId: string): Promise<CodebuddyAccount> {
  return await invoke('codebuddy_oauth_login_complete', { loginId });
}

export async function cancelCodebuddyOAuthLogin(loginId?: string): Promise<void> {
  return await invoke('codebuddy_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function addCodebuddyAccountWithToken(accessToken: string): Promise<CodebuddyAccount> {
  return await invoke('add_codebuddy_account_with_token', { accessToken });
}

export async function updateCodebuddyAccountTags(accountId: string, tags: string[]): Promise<CodebuddyAccount> {
  return await invoke('update_codebuddy_account_tags', { accountId, tags });
}

export async function getCodebuddyAccountsIndexPath(): Promise<string> {
  return await invoke('get_codebuddy_accounts_index_path');
}

export async function injectCodebuddyToVSCode(accountId: string, inlinePathRecovery = false): Promise<string> {
  return await invoke('inject_codebuddy_to_vscode', { accountId, inlinePathRecovery });
}
