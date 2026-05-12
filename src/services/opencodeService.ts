import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { OpenCodeAccount } from '../types/opencode';

export async function openOpenCodeAuthPage(): Promise<void> {
  await openUrl('https://opencode.ai/auth');
}

export async function listOpenCodeAccounts(): Promise<OpenCodeAccount[]> {
  return await invoke('list_opencode_accounts');
}

export async function addOpenCodeAccount(
  apiKey: string,
  tier: 'go' | 'zen',
): Promise<OpenCodeAccount> {
  return await invoke('add_opencode_account', { apiKey, tier });
}

export async function deleteOpenCodeAccount(accountId: string): Promise<void> {
  return await invoke('delete_opencode_account', { accountId });
}

export async function deleteOpenCodeAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_opencode_accounts', { accountIds });
}

export async function importOpenCodeFromJson(jsonContent: string): Promise<OpenCodeAccount[]> {
  return await invoke('import_opencode_from_json', { jsonContent });
}

export async function exportOpenCodeAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_opencode_accounts', { accountIds });
}

export async function refreshOpenCodeAccount(accountId: string): Promise<OpenCodeAccount> {
  return await invoke('refresh_opencode_token', { accountId });
}

export async function updateOpenCodeAccountTags(
  accountId: string,
  tags: string[],
): Promise<OpenCodeAccount> {
  return await invoke('update_opencode_account_tags', { accountId, tags });
}

export async function refreshAllOpenCodeTokens(): Promise<number> {
  return await invoke('refresh_all_opencode_tokens');
}

export async function injectOpenCodeAccount(accountId: string): Promise<string> {
  return await invoke('inject_opencode_account', { accountId });
}

export async function getOpenCodeAccountsIndexPath(): Promise<string> {
  return await invoke('get_opencode_accounts_index_path');
}
