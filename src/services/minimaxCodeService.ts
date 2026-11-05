import { invoke } from '@tauri-apps/api/core';
import type {
  MinimaxCodeAccount,
  MinimaxCodeCheckinResult,
  MinimaxCodeCheckinStatus,
} from '../types/minimaxCode';

export interface MinimaxCodeOAuthLoginStartResponse {
  loginId: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  userCode: string;
  expiresIn: number;
  intervalSeconds: number;
}

interface RawOAuthStartResponse {
  loginId?: string;
  login_id?: string;
  verificationUri?: string;
  verification_uri?: string;
  verificationUriComplete?: string | null;
  verification_uri_complete?: string | null;
  userCode?: string;
  user_code?: string;
  expiresIn?: number;
  expires_in?: number;
  intervalSeconds?: number;
  interval_seconds?: number;
}

function normalizeStartResponse(raw: RawOAuthStartResponse): MinimaxCodeOAuthLoginStartResponse {
  return {
    loginId: raw.loginId ?? raw.login_id ?? '',
    verificationUri: raw.verificationUri ?? raw.verification_uri ?? '',
    verificationUriComplete: raw.verificationUriComplete ?? raw.verification_uri_complete ?? null,
    userCode: raw.userCode ?? raw.user_code ?? '',
    expiresIn: raw.expiresIn ?? raw.expires_in ?? 0,
    intervalSeconds: raw.intervalSeconds ?? raw.interval_seconds ?? 5,
  };
}

// ---- OAuth device flow ----

export async function startMinimaxCodeOAuthLogin(
  region?: string,
): Promise<MinimaxCodeOAuthLoginStartResponse> {
  const raw = await invoke<RawOAuthStartResponse>('minimax_code_oauth_login_start', {
    region: region ?? null,
  });
  return normalizeStartResponse(raw);
}

export async function peekMinimaxCodeOAuthLogin(): Promise<MinimaxCodeOAuthLoginStartResponse | null> {
  const raw = await invoke<RawOAuthStartResponse | null>('minimax_code_oauth_login_peek');
  return raw ? normalizeStartResponse(raw) : null;
}

export async function completeMinimaxCodeOAuthLogin(loginId: string): Promise<MinimaxCodeAccount> {
  return await invoke('minimax_code_oauth_login_complete', { loginId });
}

export async function cancelMinimaxCodeOAuthLogin(loginId?: string): Promise<void> {
  await invoke('minimax_code_oauth_login_cancel', { loginId: loginId ?? null });
}

// ---- Accounts ----

export async function listMinimaxCodeAccounts(): Promise<MinimaxCodeAccount[]> {
  return await invoke('list_minimax_code_accounts');
}

export async function deleteMinimaxCodeAccount(accountId: string): Promise<void> {
  await invoke('delete_minimax_code_account', { accountId });
}

export async function deleteMinimaxCodeAccounts(accountIds: string[]): Promise<void> {
  await invoke('delete_minimax_code_accounts', { accountIds });
}

export async function importMinimaxCodeFromJson(jsonContent: string): Promise<MinimaxCodeAccount[]> {
  return await invoke('import_minimax_code_from_json', { jsonContent });
}

export async function importMinimaxCodeFromLocal(): Promise<MinimaxCodeAccount[]> {
  return await invoke('import_minimax_code_from_local');
}

export async function exportMinimaxCodeAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_minimax_code_accounts', { accountIds });
}

export async function refreshMinimaxCodeToken(accountId: string): Promise<MinimaxCodeAccount> {
  return await invoke('refresh_minimax_code_token', { accountId });
}

export async function refreshAllMinimaxCodeTokens(): Promise<number> {
  return await invoke('refresh_all_minimax_code_tokens');
}

export async function addMinimaxCodeAccountWithToken(
  accessToken: string,
  region?: string,
): Promise<MinimaxCodeAccount> {
  return await invoke('add_minimax_code_account_with_token', {
    accessToken,
    region: region ?? null,
  });
}

export async function updateMinimaxCodeAccountTags(
  accountId: string,
  tags: string[],
): Promise<MinimaxCodeAccount> {
  return await invoke('update_minimax_code_account_tags', { accountId, tags });
}

export async function getMinimaxCodeAccountsIndexPath(): Promise<string> {
  return await invoke('get_minimax_code_accounts_index_path');
}

export async function injectMinimaxCodeToClient(accountId: string): Promise<string> {
  return await invoke('inject_minimax_code_to_client', { accountId });
}

// ---- Check-in ----

export async function getCheckinStatusMinimaxCode(accountId: string): Promise<MinimaxCodeCheckinStatus> {
  return await invoke('get_checkin_status_minimax_code', { accountId });
}

export async function checkinMinimaxCode(accountId: string): Promise<MinimaxCodeCheckinResult> {
  return await invoke('checkin_minimax_code', { accountId });
}
