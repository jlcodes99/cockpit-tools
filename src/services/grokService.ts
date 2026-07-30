import { invoke } from '@tauri-apps/api/core';
import type { GrokAccount } from '../types/grok';

export interface GrokOAuthLoginStartResponse {
  loginId: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
}

export async function listGrokAccounts(): Promise<GrokAccount[]> {
  return await invoke('list_grok_accounts');
}

export async function deleteGrokAccount(accountId: string): Promise<void> {
  await invoke('delete_grok_account', { accountId });
}

export async function deleteGrokAccounts(accountIds: string[]): Promise<void> {
  await invoke('delete_grok_accounts', { accountIds });
}

export async function importGrokFromJson(
  jsonContent: string,
): Promise<GrokAccount[]> {
  return await invoke('import_grok_from_json', { jsonContent });
}

export async function addGrokAccountWithApiKey(
  apiKey: string,
  options?: {
    apiBaseUrl?: string | null;
    apiModel?: string | null;
  },
): Promise<GrokAccount> {
  return await invoke('add_grok_account_with_api_key', {
    apiKey,
    apiBaseUrl: options?.apiBaseUrl?.trim() || null,
    apiModel: options?.apiModel?.trim() || null,
  });
}

export async function importGrokFromLocal(): Promise<GrokAccount[]> {
  return await invoke('import_grok_from_local');
}

export async function exportGrokAccounts(
  accountIds: string[],
): Promise<string> {
  return await invoke('export_grok_accounts', { accountIds });
}

export async function startGrokOAuthLogin(): Promise<GrokOAuthLoginStartResponse> {
  return await invoke('grok_oauth_login_start');
}

export async function completeGrokOAuthLogin(
  loginId: string,
  reauthAccountId?: string | null,
): Promise<GrokAccount> {
  return await invoke('grok_oauth_login_complete', {
    loginId,
    reauthAccountId: reauthAccountId ?? null,
  });
}

export async function cancelGrokOAuthLogin(loginId?: string): Promise<void> {
  await invoke('grok_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function refreshGrokAccount(
  accountId: string,
): Promise<GrokAccount> {
  return await invoke('refresh_grok_account', { accountId });
}

export async function refreshAllGrokAccounts(): Promise<number> {
  return await invoke('refresh_all_grok_accounts');
}

export async function switchGrokAccount(accountId: string): Promise<string> {
  return await invoke('switch_grok_account', { accountId });
}

export async function updateGrokAccountTags(
  accountId: string,
  tags: string[],
): Promise<GrokAccount> {
  return await invoke('update_grok_account_tags', { accountId, tags });
}

export async function updateGrokAccountWorkingDir(
  accountId: string,
  workingDir?: string | null,
): Promise<GrokAccount> {
  return await invoke('update_grok_account_working_dir', {
    accountId,
    workingDir: workingDir?.trim() || null,
  });
}

export async function getGrokCurrentAccountId(): Promise<string | null> {
  return await invoke('get_grok_current_account_id');
}

export async function getGrokAccountsIndexPath(): Promise<string> {
  return await invoke('get_grok_accounts_index_path');
}
