import { invoke } from '@tauri-apps/api/core';
import type { ZcodeAccount } from '../types/zcode';

export type ZcodeOAuthProvider = 'zai' | 'bigmodel';
export type ZcodeApiKeyProvider = 'zai' | 'bigmodel';

export interface ZcodeOAuthStartResponse {
  loginId: string;
  provider: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl: string;
}

export async function listZcodeAccounts(): Promise<ZcodeAccount[]> {
  return await invoke('list_zcode_accounts');
}

export async function deleteZcodeAccount(accountId: string): Promise<void> {
  await invoke('delete_zcode_account', { accountId });
}

export async function deleteZcodeAccounts(accountIds: string[]): Promise<void> {
  await invoke('delete_zcode_accounts', { accountIds });
}

export async function importZcodeFromJson(jsonContent: string): Promise<ZcodeAccount[]> {
  return await invoke('import_zcode_from_json', { jsonContent });
}

export async function importZcodeFromFile(path: string): Promise<ZcodeAccount[]> {
  return await invoke('import_zcode_from_file', { path });
}

export async function importZcodeFromLocal(): Promise<ZcodeAccount[]> {
  return await invoke('import_zcode_from_local');
}

export async function importZcodeApiKey(
  apiKey: string,
  provider: ZcodeApiKeyProvider,
  accountName?: string,
): Promise<ZcodeAccount> {
  return await invoke('import_zcode_api_key', {
    apiKey,
    provider,
    accountName: accountName?.trim() || null,
  });
}

export async function exportZcodeAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_zcode_accounts', { accountIds });
}

export async function startZcodeOAuthLogin(
  provider: ZcodeOAuthProvider,
): Promise<ZcodeOAuthStartResponse> {
  return await invoke('zcode_oauth_login_start', { provider });
}

export async function completeZcodeOAuthLogin(loginId: string): Promise<ZcodeAccount> {
  return await invoke('zcode_oauth_login_complete', { loginId });
}

export async function submitZcodeOAuthCallbackUrl(
  loginId: string,
  callbackUrl: string,
): Promise<void> {
  await invoke('zcode_oauth_submit_callback_url', { loginId, callbackUrl });
}

export async function openZcodeOAuthWindow(authUrl: string, incognito = false): Promise<void> {
  await invoke('zcode_oauth_open_window', { authUrl, incognito });
}

export async function cancelZcodeOAuthLogin(loginId?: string): Promise<void> {
  await invoke('zcode_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function refreshZcodeAccount(accountId: string): Promise<ZcodeAccount> {
  return await invoke('refresh_zcode_account', { accountId });
}

export async function refreshAllZcodeAccounts(): Promise<number> {
  return await invoke('refresh_all_zcode_accounts');
}

export async function injectZcodeAccount(accountId: string): Promise<string> {
  return await invoke('inject_zcode_account', { accountId });
}

export async function updateZcodeAccountTags(
  accountId: string,
  tags: string[],
): Promise<ZcodeAccount> {
  return await invoke('update_zcode_account_tags', { accountId, tags });
}

export async function getZcodeCurrentAccountId(): Promise<string | null> {
  return await invoke('get_zcode_current_account_id');
}

export async function getZcodeAccountsIndexPath(): Promise<string> {
  return await invoke('get_zcode_accounts_index_path');
}
