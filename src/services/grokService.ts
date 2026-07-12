import { invoke } from '@tauri-apps/api/core';
import type { GrokAccount, GrokOAuthStartResponse } from '../types/grok';

type GrokOAuthStartResponseRaw = Partial<GrokOAuthStartResponse> & {
  login_id?: string;
  verification_uri?: string;
  verification_uri_complete?: string | null;
  verificationUriComplete?: string | null;
  user_code?: string | null;
  expires_in?: number;
  interval_seconds?: number;
  callback_url?: string | null;
};

function normalizeOAuthStart(raw: GrokOAuthStartResponseRaw): GrokOAuthStartResponse {
  const loginId = raw.loginId ?? raw.login_id ?? '';
  const verificationUri =
    raw.verificationUriComplete ??
    raw.verification_uri_complete ??
    raw.verificationUri ??
    raw.verification_uri ??
    '';
  const expiresIn = Number(raw.expiresIn ?? raw.expires_in ?? 0);
  const intervalSeconds = Number(raw.intervalSeconds ?? raw.interval_seconds ?? 0);
  const userCode = raw.userCode ?? raw.user_code ?? null;
  const callbackUrl = raw.callbackUrl ?? raw.callback_url ?? null;
  if (!loginId || !verificationUri) {
    throw new Error('Grok OAuth start 响应缺少关键字段');
  }
  return {
    loginId,
    verificationUri,
    userCode,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : 600,
    intervalSeconds:
      Number.isFinite(intervalSeconds) && intervalSeconds > 0 ? intervalSeconds : 5,
    callbackUrl,
  };
}

export async function listGrokAccounts(): Promise<GrokAccount[]> {
  return await invoke('list_grok_accounts');
}

export async function getCurrentGrokAccount(): Promise<GrokAccount | null> {
  return await invoke('get_current_grok_account');
}

export async function deleteGrokAccount(accountId: string): Promise<void> {
  return await invoke('delete_grok_account', { accountId });
}

export async function deleteGrokAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_grok_accounts', { accountIds });
}

export async function importGrokFromJson(jsonContent: string): Promise<GrokAccount[]> {
  return await invoke('import_grok_from_json', { jsonContent });
}

export async function importGrokFromLocal(): Promise<GrokAccount[]> {
  return await invoke('import_grok_from_local');
}

export async function exportGrokAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_grok_accounts', { accountIds });
}

export async function refreshGrokToken(accountId: string): Promise<GrokAccount> {
  return await invoke('refresh_grok_token', { accountId });
}

export async function refreshAllGrokTokens(): Promise<number> {
  return await invoke('refresh_all_grok_tokens');
}

export async function updateGrokAccountTags(
  accountId: string,
  tags: string[],
): Promise<GrokAccount> {
  return await invoke('update_grok_account_tags', { accountId, tags });
}

export async function getGrokAccountsIndexPath(): Promise<string> {
  return await invoke('get_grok_accounts_index_path');
}

export async function getGrokAuthJsonPath(): Promise<string> {
  return await invoke('get_grok_auth_json_path');
}

export async function switchGrokAccount(accountId: string): Promise<GrokAccount> {
  return await invoke('switch_grok_account', { accountId });
}

export async function injectGrokAccount(accountId: string): Promise<string> {
  return await invoke('inject_grok_account', { accountId });
}

export async function startGrokOAuthLogin(): Promise<GrokOAuthStartResponse> {
  const raw = await invoke<GrokOAuthStartResponseRaw>('grok_oauth_login_start');
  return normalizeOAuthStart(raw);
}

export async function completeGrokOAuthLogin(loginId: string): Promise<GrokAccount> {
  return await invoke('grok_oauth_login_complete', { loginId });
}

export async function cancelGrokOAuthLogin(loginId?: string): Promise<void> {
  return await invoke('grok_oauth_login_cancel', { loginId: loginId ?? null });
}
