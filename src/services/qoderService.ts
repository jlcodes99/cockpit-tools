import { invoke } from '@tauri-apps/api/core';
import { QoderAccount } from '../types/qoder';
import type { QoderSwitchOutcome } from '../types/qoder';
import type { QoderRegion, QoderClient } from '../utils/qoderRegion';

export function openQoderClient(
  region: QoderRegion,
  client: QoderClient,
  options?: { workingDir?: string; terminal?: string; instanceId?: string },
): Promise<void> {
  return invoke('qoder_open_client', { region, client, ...options });
}

export interface QoderOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

export type QoderOAuthStartResponseRaw = Partial<QoderOAuthStartResponse> & {
  login_id?: string;
  verification_uri?: string;
  expires_in?: number;
  interval_seconds?: number;
  callback_url?: string | null;
};

export function normalizeQoderOAuthStartResponse(raw: QoderOAuthStartResponseRaw): QoderOAuthStartResponse {
  const loginId = raw.loginId ?? raw.login_id ?? '';
  const verificationUri = raw.verificationUri ?? raw.verification_uri ?? '';
  const expiresIn = Number(raw.expiresIn ?? raw.expires_in ?? 0);
  const intervalSeconds = Number(raw.intervalSeconds ?? raw.interval_seconds ?? 0);
  const callbackUrl = raw.callbackUrl ?? raw.callback_url ?? null;

  if (!loginId || !verificationUri) {
    throw new Error('Qoder OAuth start 响应缺少关键字段');
  }

  return {
    loginId,
    verificationUri,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : 600,
    intervalSeconds: Number.isFinite(intervalSeconds) && intervalSeconds > 0 ? intervalSeconds : 1,
    callbackUrl,
  };
}

export async function listQoderAccounts(): Promise<QoderAccount[]> {
  return await invoke('list_qoder_accounts');
}

export async function deleteQoderAccount(accountId: string): Promise<void> {
  return await invoke('delete_qoder_account', { accountId });
}

export async function deleteQoderAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_qoder_accounts', { accountIds });
}

export async function importQoderFromJson(
  jsonContent: string,
  // Accepted so the shared page can pass its region uniformly; the global
  // command always resolves to the global store.
  _region?: QoderRegion,
): Promise<QoderAccount[]> {
  return await invoke('import_qoder_from_json', { jsonContent });
}

export async function importQoderFromLocal(): Promise<QoderAccount[]> {
  return await invoke('import_qoder_from_local');
}

/**
 * Reads the credentials the installed global client holds. The desktop and CLI
 * clients keep their own files, separate from the IDE database the `ide` target
 * uses, so a client has to be named explicitly.
 */
export async function importQoderNative(
  region: QoderRegion,
  client: QoderClient,
): Promise<QoderAccount[]> {
  return await invoke('import_qoder_native', { region, client });
}

/**
 * Writes one account into the installed global client.
 */
export async function injectQoderNative(
  accountId: string,
  client: QoderClient,
  options: { launch?: boolean; workingDir?: string; terminal?: string } = {},
): Promise<QoderSwitchOutcome> {
  return await invoke('inject_qoder_native', { accountId, client, ...options });
}

export async function qoderOauthLoginStart(_region?: QoderRegion): Promise<QoderOAuthStartResponse> {
  const raw = await invoke<QoderOAuthStartResponseRaw>('qoder_oauth_login_start');
  return normalizeQoderOAuthStartResponse(raw);
}

export async function qoderOauthLoginPeek(): Promise<QoderOAuthStartResponse | null> {
  const raw = await invoke<QoderOAuthStartResponseRaw | null>('qoder_oauth_login_peek');
  if (!raw) return null;
  try {
    return normalizeQoderOAuthStartResponse(raw);
  } catch {
    return null;
  }
}

export async function qoderOauthLoginComplete(loginId: string): Promise<QoderAccount> {
  return await invoke('qoder_oauth_login_complete', { loginId });
}

export async function qoderOauthLoginCancel(loginId?: string): Promise<void> {
  return await invoke('qoder_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function exportQoderAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_qoder_accounts', { accountIds });
}

export async function refreshQoderToken(accountId: string): Promise<QoderAccount> {
  return await invoke('refresh_qoder_token', { accountId });
}

export async function refreshAllQoderTokens(): Promise<number> {
  return await invoke('refresh_all_qoder_tokens');
}

export async function injectQoderAccount(accountId: string): Promise<string> {
  return await invoke('inject_qoder_account', { accountId });
}

export async function updateQoderAccountTags(
  accountId: string,
  tags: string[],
): Promise<QoderAccount> {
  return await invoke('update_qoder_account_tags', { accountId, tags });
}

export async function getQoderAccountsIndexPath(): Promise<string> {
  return await invoke('get_qoder_accounts_index_path');
}
