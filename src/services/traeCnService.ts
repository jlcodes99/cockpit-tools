import { invoke } from '@tauri-apps/api/core';
import type { TraeAccount } from '../types/trae';
import type { TraeOAuthStartResponse, TraeOAuthStartResponseRaw } from './traeService';
import { normalizeTraeOAuthStartResponse } from './traeService';

export type TraeCnLaunchOnSwitchConfig = {
  enabled: boolean;
};

export async function listTraeCnAccounts(): Promise<TraeAccount[]> {
  return await invoke('list_trae_cn_accounts');
}

export async function deleteTraeCnAccount(accountId: string): Promise<void> {
  return await invoke('delete_trae_cn_account', { accountId });
}

export async function deleteTraeCnAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_trae_cn_accounts', { accountIds });
}

export async function importTraeCnFromJson(jsonContent: string): Promise<TraeAccount[]> {
  return await invoke('import_trae_cn_from_json', { jsonContent });
}

export async function importTraeCnFromLocal(): Promise<TraeAccount[]> {
  return await invoke('import_trae_cn_from_local');
}

export async function traeCnOauthLoginStart(): Promise<TraeOAuthStartResponse> {
  const raw = await invoke<TraeOAuthStartResponseRaw>('trae_cn_oauth_login_start');
  return normalizeTraeOAuthStartResponse(raw);
}

export async function traeCnOauthLoginComplete(loginId: string): Promise<TraeAccount> {
  return await invoke('trae_cn_oauth_login_complete', { loginId });
}

export async function traeCnOauthLoginCancel(loginId?: string): Promise<void> {
  return await invoke('trae_cn_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function traeCnOauthSubmitCallbackUrl(
  loginId: string,
  callbackUrl: string,
): Promise<void> {
  return await invoke('trae_cn_oauth_submit_callback_url', { loginId, callbackUrl });
}

export async function exportTraeCnAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_trae_cn_accounts', { accountIds });
}

export async function refreshTraeCnToken(accountId: string): Promise<TraeAccount> {
  return await invoke('refresh_trae_cn_token', { accountId });
}

export async function refreshAllTraeCnTokens(): Promise<number> {
  return await invoke('refresh_all_trae_cn_tokens');
}

export async function addTraeCnAccountWithToken(accessToken: string): Promise<TraeAccount> {
  return await invoke('add_trae_cn_account_with_token', { accessToken });
}

export async function updateTraeCnAccountTags(accountId: string, tags: string[]): Promise<TraeAccount> {
  return await invoke('update_trae_cn_account_tags', { accountId, tags });
}

export async function getTraeCnAccountsIndexPath(): Promise<string> {
  return await invoke('get_trae_cn_accounts_index_path');
}

export async function getTraeCnLaunchOnSwitch(): Promise<TraeCnLaunchOnSwitchConfig> {
  return await invoke('get_trae_cn_launch_on_switch');
}

export async function setTraeCnLaunchOnSwitch(enabled: boolean): Promise<TraeCnLaunchOnSwitchConfig> {
  return await invoke('set_trae_cn_launch_on_switch', { enabled });
}

export async function injectTraeCnAccount(accountId: string): Promise<string> {
  return await invoke('inject_trae_cn_account', { accountId });
}
