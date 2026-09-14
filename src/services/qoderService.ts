import { invoke } from '@tauri-apps/api/core';
import { QoderAccount, QoderChannel } from '../types/qoder';

export interface QoderOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

type QoderOAuthStartResponseRaw = Partial<QoderOAuthStartResponse> & {
  login_id?: string;
  verification_uri?: string;
  expires_in?: number;
  interval_seconds?: number;
  callback_url?: string | null;
};

function normalizeQoderOAuthStartResponse(raw: QoderOAuthStartResponseRaw): QoderOAuthStartResponse {
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

export async function listQoderChannelAccounts(channel: QoderChannel = 'qoder'): Promise<QoderAccount[]> {
  return await invoke('list_qoder_channel_accounts', { channel });
}

export async function listQoderAccounts(): Promise<QoderAccount[]> {
  return await listQoderChannelAccounts('qoder');
}

export async function deleteQoderChannelAccount(channel: QoderChannel, accountId: string): Promise<void> {
  return await invoke('delete_qoder_channel_account', { channel, accountId });
}

export async function deleteQoderAccount(accountId: string): Promise<void> {
  return await deleteQoderChannelAccount('qoder', accountId);
}

export async function deleteQoderChannelAccounts(channel: QoderChannel, accountIds: string[]): Promise<void> {
  return await invoke('delete_qoder_channel_accounts', { channel, accountIds });
}

export async function deleteQoderAccounts(accountIds: string[]): Promise<void> {
  return await deleteQoderChannelAccounts('qoder', accountIds);
}

export async function importQoderChannelFromJson(channel: QoderChannel, jsonContent: string): Promise<QoderAccount[]> {
  return await invoke('import_qoder_channel_from_json', { channel, jsonContent });
}

export async function importQoderFromJson(jsonContent: string): Promise<QoderAccount[]> {
  return await importQoderChannelFromJson('qoder', jsonContent);
}

export async function importQoderChannelFromLocal(channel: QoderChannel): Promise<QoderAccount[]> {
  return await invoke('import_qoder_channel_from_local', { channel });
}

export async function importQoderFromLocal(): Promise<QoderAccount[]> {
  return await importQoderChannelFromLocal('qoder');
}

export async function exportQoderChannelAccounts(channel: QoderChannel, accountIds: string[]): Promise<string> {
  return await invoke('export_qoder_channel_accounts', { channel, accountIds });
}

export async function exportQoderAccounts(accountIds: string[]): Promise<string> {
  return await exportQoderChannelAccounts('qoder', accountIds);
}

export async function injectQoderChannelAccount(channel: QoderChannel, accountId: string): Promise<string> {
  return await invoke('inject_qoder_channel_account', { channel, accountId });
}

export async function injectQoderAccount(accountId: string): Promise<string> {
  return await injectQoderChannelAccount('qoder', accountId);
}

export async function updateQoderChannelAccountTags(
  channel: QoderChannel,
  accountId: string,
  tags: string[],
): Promise<QoderAccount> {
  return await invoke('update_qoder_channel_account_tags', { channel, accountId, tags });
}

export async function updateQoderAccountTags(
  accountId: string,
  tags: string[],
): Promise<QoderAccount> {
  return await updateQoderChannelAccountTags('qoder', accountId, tags);
}

export async function getQoderChannelAccountsIndexPath(channel: QoderChannel): Promise<string> {
  return await invoke('get_qoder_channel_accounts_index_path', { channel });
}

export async function getQoderAccountsIndexPath(): Promise<string> {
  return await getQoderChannelAccountsIndexPath('qoder');
}

export async function qoderOauthLoginStart(): Promise<QoderOAuthStartResponse> {
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

export async function refreshQoderToken(accountId: string, channel?: string): Promise<QoderAccount> {
  return await invoke('refresh_qoder_token', { accountId, channel: channel ?? null });
}

export async function refreshAllQoderTokens(): Promise<number> {
  return await invoke('refresh_all_qoder_tokens');
}
