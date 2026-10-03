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

export interface PiGatewayAccountInput {
  provider: string;
  name?: string | null;
  baseUrl: string;
  api: string;
  authHeader: boolean;
  apiKey: string;
  models: string[];
  displayName?: string | null;
  defaultModel?: string | null;
}

/** Add a third-party gateway (custom models.json provider + auth.json key). */
export async function addPiAccountWithGateway(
  input: PiGatewayAccountInput,
): Promise<PiAccount> {
  return await invoke('add_pi_account_with_gateway', {
    provider: input.provider.trim(),
    name: input.name?.trim() || null,
    baseUrl: input.baseUrl.trim(),
    api: input.api,
    authHeader: input.authHeader,
    apiKey: input.apiKey,
    models: input.models,
    displayName: input.displayName?.trim() || null,
    defaultModel: input.defaultModel?.trim() || null,
  });
}

export async function listPiGatewayModels(input: {
  baseUrl: string;
  api: string;
  apiKey: string;
  authHeader: boolean;
  /** When apiKey is blank, reuse the key stored on this account/provider. */
  accountId?: string;
  provider?: string;
}): Promise<string[]> {
  return await invoke('pi_list_gateway_models', {
    baseUrl: input.baseUrl.trim(),
    api: input.api,
    apiKey: input.apiKey.trim(),
    authHeader: input.authHeader,
    accountId: input.accountId ?? null,
    provider: input.provider ?? null,
  });
}

/** Edit a gateway provider; a blank apiKey keeps the stored key. */
export async function updatePiAccountGateway(
  accountId: string,
  input: PiGatewayAccountInput,
): Promise<PiAccount> {
  return await invoke('update_pi_account_gateway', {
    accountId,
    provider: input.provider.trim(),
    name: input.name?.trim() || null,
    baseUrl: input.baseUrl.trim(),
    api: input.api,
    authHeader: input.authHeader,
    apiKey: input.apiKey.trim() || null,
    models: input.models,
    displayName: input.displayName?.trim() || null,
    defaultModel: input.defaultModel?.trim() || null,
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

/** Refresh expired tokens and usage for one account. */
export async function refreshPiAccount(accountId: string): Promise<void> {
  await queryPiAccountUsage(accountId);
}

/** Refresh every account; resolves to the number that succeeded. */
export async function refreshAllPiAccounts(): Promise<number> {
  const accounts = await listPiAccounts();
  const results = await Promise.allSettled(
    accounts.map((account) => queryPiAccountUsage(account.id)),
  );
  return results.filter((result) => result.status === 'fulfilled').length;
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

export interface PiUsageWindow {
  key: string;
  label: string;
  used_percent: number;
  reset_at?: number;
  detail?: string;
}

export interface PiProviderUsage {
  provider: string;
  plan?: string;
  windows: PiUsageWindow[];
  balance?: string;
  gateway?: import('./modelProviderUsageService').ModelProviderUsageSummary;
  error?: string;
  unsupported?: boolean;
}

/** Query usage for every credential on a pi account (credentials are not refreshed). */
export async function queryPiAccountUsage(accountId: string): Promise<PiProviderUsage[]> {
  return await invoke('pi_query_account_usage', { accountId });
}
