import { invoke } from '@tauri-apps/api/core';
import type { PiAccount } from '../types/pi';

export async function listPiAccounts(): Promise<PiAccount[]> {
  return await invoke('list_pi_accounts');
}

export async function deletePiAccount(accountId: string): Promise<void> {
  await invoke('delete_pi_account', { accountId });
}

export async function deletePiAccounts(accountIds: string[]): Promise<void> {
  await invoke('delete_pi_accounts', { accountIds });
}

export async function importPiFromJson(jsonContent: string): Promise<PiAccount[]> {
  return await invoke('import_pi_from_json', { jsonContent });
}

export async function addPiAccountWithApiKey(
  provider: string,
  apiKey: string,
  options?: { displayName?: string | null; defaultModel?: string | null },
): Promise<PiAccount> {
  return await invoke('add_pi_account_with_api_key', {
    provider: provider.trim(),
    apiKey,
    displayName: options?.displayName?.trim() || null,
    defaultModel: options?.defaultModel?.trim() || null,
  });
}

export async function importPiFromLocal(): Promise<PiAccount[]> {
  return await invoke('import_pi_from_local');
}

export async function exportPiAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_pi_accounts', { accountIds });
}

export async function switchPiAccount(accountId: string): Promise<string> {
  return await invoke('switch_pi_account', { accountId });
}

export async function updatePiAccountTags(
  accountId: string,
  tags: string[],
): Promise<PiAccount> {
  return await invoke('update_pi_account_tags', { accountId, tags });
}

export async function updatePiAccountWorkingDir(
  accountId: string,
  workingDir?: string | null,
): Promise<PiAccount> {
  return await invoke('update_pi_account_working_dir', {
    accountId,
    workingDir: workingDir?.trim() || null,
  });
}

export async function updatePiAccountDefaults(
  accountId: string,
  defaults: {
    displayName?: string | null;
    defaultProvider?: string | null;
    defaultModel?: string | null;
    defaultThinkingLevel?: string | null;
  },
): Promise<PiAccount> {
  return await invoke('update_pi_account_defaults', {
    accountId,
    displayName: defaults.displayName?.trim() || null,
    defaultProvider: defaults.defaultProvider?.trim() || null,
    defaultModel: defaults.defaultModel?.trim() || null,
    defaultThinkingLevel: defaults.defaultThinkingLevel?.trim() || null,
  });
}

export async function getPiCurrentAccountId(): Promise<string | null> {
  return await invoke('get_pi_current_account_id');
}

export async function getPiAccountsIndexPath(): Promise<string> {
  return await invoke('get_pi_accounts_index_path');
}

/** No remote refresh yet; reload the local account list. */
export async function refreshPiAccount(_accountId: string): Promise<void> {
  await listPiAccounts();
}

export async function refreshAllPiAccounts(): Promise<number> {
  return (await listPiAccounts()).length;
}

export type PiOAuthProvider =
  | 'anthropic'
  | 'openai-codex'
  | 'github-copilot'
  | 'kimi-coding'
  | 'xai'
  | 'openrouter';

export interface PiOAuthStartResponse {
  loginId: string;
  provider: string;
  authUrl: string;
  verificationUri: string;
  /** Device-code flows (Copilot / Kimi / xAI) only. */
  userCode?: string;
  expiresIn: number;
  intervalSeconds: number;
  /** Absent for device-code flows. */
  callbackUrl?: string | null;
  /** False when the callback port was busy; the redirect URL must be pasted. */
  listening: boolean;
}

/** Start pi's built-in login for `provider` (PKCE callback or device code). */
export async function startPiOAuthLogin(provider: PiOAuthProvider): Promise<PiOAuthStartResponse> {
  return await invoke('pi_oauth_login_start', { provider });
}

/** Resolves once the browser callback (or a pasted URL) completes the login. */
export async function completePiOAuthLogin(loginId: string): Promise<PiAccount> {
  return await invoke('pi_oauth_login_complete', { loginId });
}

export async function cancelPiOAuthLogin(loginId?: string): Promise<void> {
  await invoke('pi_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function submitPiOAuthCallbackUrl(loginId: string, callbackUrl: string): Promise<void> {
  await invoke('pi_oauth_submit_callback', { loginId, callbackUrl });
}
