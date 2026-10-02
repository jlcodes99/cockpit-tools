import { invoke } from '@tauri-apps/api/core';
import { ZcodeAccount } from '../types/zcode';

export async function listZcodeAccounts(): Promise<ZcodeAccount[]> {
  return await invoke('list_zcode_accounts');
}

export async function deleteZcodeAccount(accountId: string): Promise<void> {
  return await invoke('delete_zcode_account', { accountId });
}

export async function deleteZcodeAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_zcode_accounts', { accountIds });
}

export async function importZcodeFromLocal(): Promise<ZcodeAccount[]> {
  return await invoke('import_zcode_from_local');
}

export async function importZcodeFromJson(jsonContent: string): Promise<ZcodeAccount[]> {
  return await invoke('import_zcode_from_json', { jsonContent });
}

export async function exportZcodeAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_zcode_accounts', { accountIds });
}

export async function refreshZcodeQuota(accountId: string): Promise<ZcodeAccount> {
  return await invoke('refresh_zcode_quota', { accountId });
}

export async function refreshAllZcodeQuotas(): Promise<number> {
  return await invoke('refresh_all_zcode_quotas');
}

export async function updateZcodeAccountTags(accountId: string, tags: string[]): Promise<ZcodeAccount> {
  return await invoke('update_zcode_account_tags', { accountId, tags });
}

// ZCode 凭证只读（仅额度展示），不支持在应用内切换账号
export async function injectZcodeAccount(): Promise<never> {
  throw new Error('ZCode 暂不支持切换账号');
}
