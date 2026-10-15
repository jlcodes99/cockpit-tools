import { invoke } from '@tauri-apps/api/core';
import type {
  ClaudeAccount,
  ClaudeDesktopGatewayConnectionMode,
  ClaudeDesktopGatewayModelMapping,
  ClaudeDesktopGatewayModelsResult,
  ClaudeDesktopLoginStartResponse,
  ClaudeOAuthStartResponse,
} from '../types/claude';

type ClaudeDesktopLoginStartResponseRaw = Partial<ClaudeDesktopLoginStartResponse> & {
  login_id?: string;
  user_data_dir?: string;
  expires_in?: number;
  interval_seconds?: number;
};

type ClaudeOAuthStartResponseRaw = Partial<ClaudeOAuthStartResponse> & {
  login_id?: string;
  verification_uri?: string;
  expires_in?: number;
  interval_seconds?: number;
};

export interface ClaudeCliLaunchInfo {
  accountId: string;
  accountEmail: string;
  workingDir: string;
  launchCommand: string;
}

function normalizeClaudeDesktopLoginStartResponse(
  raw: ClaudeDesktopLoginStartResponseRaw,
): ClaudeDesktopLoginStartResponse {
  const loginId = raw.loginId ?? raw.login_id ?? '';
  const userDataDir = raw.userDataDir ?? raw.user_data_dir ?? '';
  const expiresIn = Number(raw.expiresIn ?? raw.expires_in ?? 0);
  const intervalSeconds = Number(raw.intervalSeconds ?? raw.interval_seconds ?? 0);

  if (!loginId || !userDataDir) {
    throw new Error('Claude login start 响应缺少关键字段');
  }

  return {
    loginId,
    userDataDir,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : 1800,
    intervalSeconds: Number.isFinite(intervalSeconds) && intervalSeconds > 0 ? intervalSeconds : 2,
  };
}

function normalizeClaudeOAuthStartResponse(raw: ClaudeOAuthStartResponseRaw): ClaudeOAuthStartResponse {
  const loginId = raw.loginId ?? raw.login_id ?? '';
  const verificationUri = raw.verificationUri ?? raw.verification_uri ?? '';
  const expiresIn = Number(raw.expiresIn ?? raw.expires_in ?? 0);
  const intervalSeconds = Number(raw.intervalSeconds ?? raw.interval_seconds ?? 0);

  if (!loginId || !verificationUri) {
    throw new Error('Claude OAuth start 响应缺少关键字段');
  }

  return {
    loginId,
    verificationUri,
    expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : 600,
    intervalSeconds: Number.isFinite(intervalSeconds) && intervalSeconds > 0 ? intervalSeconds : 1,
  };
}

export async function listClaudeAccounts(): Promise<ClaudeAccount[]> {
  return await invoke('list_claude_accounts');
}

export async function deleteClaudeAccount(accountId: string): Promise<void> {
  return await invoke('delete_claude_account', { accountId });
}

export async function deleteClaudeAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_claude_accounts', { accountIds });
}

export async function uninstallClaudeDesktopLoginComponent(): Promise<void> {
  return await invoke('claude_uninstall_desktop_login_component');
}

export async function getClaudeDesktopLoginComponentStorage(): Promise<{ sizeBytes: number }> {
  const raw = await invoke<{ sizeBytes?: number; size_bytes?: number }>(
    'claude_get_desktop_login_component_storage',
  );
  const sizeBytes = Number(raw.sizeBytes ?? raw.size_bytes ?? 0);
  return { sizeBytes: Number.isFinite(sizeBytes) && sizeBytes >= 0 ? sizeBytes : 0 };
}

export async function importClaudeFromJson(jsonContent: string): Promise<ClaudeAccount[]> {
  return await invoke('import_claude_from_json', { jsonContent });
}

export async function importClaudeApiKey(
  apiKey: string,
  accountName?: string,
  provider?: {
    apiBaseUrl?: string | null;
    apiProviderId?: string | null;
    apiProviderName?: string | null;
    apiProviderSourceTag?: string | null;
    apiProviderWebsite?: string | null;
    apiProviderApiKeyUrl?: string | null;
    apiKeyField?: string | null;
    apiModelCatalog?: string[] | null;
    apiExtraEnv?: Record<string, string> | null;
  },
): Promise<ClaudeAccount> {
  return await invoke('import_claude_api_key', {
    apiKey,
    accountName: accountName?.trim() || null,
    apiBaseUrl: provider?.apiBaseUrl?.trim() || null,
    apiProviderId: provider?.apiProviderId?.trim() || null,
    apiProviderName: provider?.apiProviderName?.trim() || null,
    apiProviderSourceTag: provider?.apiProviderSourceTag?.trim() || null,
    apiProviderWebsite: provider?.apiProviderWebsite?.trim() || null,
    apiProviderApiKeyUrl: provider?.apiProviderApiKeyUrl?.trim() || null,
    apiKeyField: provider?.apiKeyField?.trim() || null,
    apiModelCatalog: provider?.apiModelCatalog ?? null,
    apiExtraEnv: provider?.apiExtraEnv ?? null,
  });
}

type ClaudeDesktopGatewayProviderInput = {
  apiBaseUrl?: string | null;
  apiProviderId?: string | null;
  apiProviderName?: string | null;
  apiProviderSourceTag?: string | null;
  apiProviderWebsite?: string | null;
  apiProviderApiKeyUrl?: string | null;
  apiKeyField?: string | null;
  apiModelCatalog?: string[] | null;
  apiExtraEnv?: Record<string, string> | null;
  authScheme?: string | null;
  desktopGatewayModels?: string[] | null;
  desktopGatewayConnectionMode?: ClaudeDesktopGatewayConnectionMode | string | null;
  desktopGatewayUpstreamModels?: string[] | null;
  desktopGatewayModelMappings?: ClaudeDesktopGatewayModelMapping[] | null;
};

function buildClaudeDesktopGatewayPayload(
  apiKey: string,
  accountName?: string,
  provider?: ClaudeDesktopGatewayProviderInput,
) {
  return {
    apiKey,
    accountName: accountName?.trim() || null,
    apiBaseUrl: provider?.apiBaseUrl?.trim() || null,
    apiProviderId: provider?.apiProviderId?.trim() || null,
    apiProviderName: provider?.apiProviderName?.trim() || null,
    apiProviderSourceTag: provider?.apiProviderSourceTag?.trim() || null,
    apiProviderWebsite: provider?.apiProviderWebsite?.trim() || null,
    apiProviderApiKeyUrl: provider?.apiProviderApiKeyUrl?.trim() || null,
    apiKeyField: provider?.apiKeyField?.trim() || null,
    apiModelCatalog: provider?.apiModelCatalog ?? null,
    apiExtraEnv: provider?.apiExtraEnv ?? null,
    authScheme: provider?.authScheme?.trim() || null,
    desktopGatewayModels: provider?.desktopGatewayModels ?? null,
    desktopGatewayConnectionMode: provider?.desktopGatewayConnectionMode?.trim() || null,
    desktopGatewayUpstreamModels: provider?.desktopGatewayUpstreamModels ?? null,
    desktopGatewayModelMappings: provider?.desktopGatewayModelMappings ?? null,
  };
}

export async function importClaudeDesktopGateway(
  apiKey: string,
  accountName?: string,
  provider?: ClaudeDesktopGatewayProviderInput,
): Promise<ClaudeAccount> {
  return await invoke('import_claude_desktop_gateway', buildClaudeDesktopGatewayPayload(apiKey, accountName, provider));
}

export async function updateClaudeDesktopGateway(
  accountId: string,
  apiKey: string,
  accountName?: string,
  provider?: ClaudeDesktopGatewayProviderInput,
): Promise<ClaudeAccount> {
  return await invoke('update_claude_desktop_gateway', {
    accountId,
    ...buildClaudeDesktopGatewayPayload(apiKey, accountName, provider),
  });
}

export async function listClaudeDesktopGatewayModels(input: {
  apiKey: string;
  apiBaseUrl: string;
  authScheme?: string | null;
}): Promise<ClaudeDesktopGatewayModelsResult> {
  return await invoke('claude_desktop_gateway_list_models', {
    apiKey: input.apiKey,
    apiBaseUrl: input.apiBaseUrl,
    authScheme: input.authScheme?.trim() || null,
  });
}

export async function importClaudeCliFromLocal(): Promise<ClaudeAccount> {
  return await invoke('import_claude_cli_from_local');
}

export async function claudeDesktopLoginStart(progressId?: string): Promise<ClaudeDesktopLoginStartResponse> {
  const raw = await invoke<ClaudeDesktopLoginStartResponseRaw>('claude_desktop_login_start', {
    progressId: progressId || null,
  });
  return normalizeClaudeDesktopLoginStartResponse(raw);
}

export async function claudeDesktopLoginComplete(
  loginId: string,
  accountName?: string,
): Promise<ClaudeAccount> {
  return await invoke('claude_desktop_login_complete', {
    loginId,
    accountName: accountName?.trim() || null,
  });
}

export async function claudeDesktopLoginCancel(loginId?: string): Promise<void> {
  return await invoke('claude_desktop_login_cancel', { loginId: loginId ?? null });
}

export async function claudeOauthLoginStart(): Promise<ClaudeOAuthStartResponse> {
  const raw = await invoke<ClaudeOAuthStartResponseRaw>('claude_oauth_login_start');
  return normalizeClaudeOAuthStartResponse(raw);
}

export async function claudeOauthLoginPrepare(): Promise<ClaudeOAuthStartResponse> {
  const raw = await invoke<ClaudeOAuthStartResponseRaw>('claude_oauth_login_prepare');
  return normalizeClaudeOAuthStartResponse(raw);
}

export async function claudeOauthLoginComplete(
  loginId: string,
  callbackOrCode: string,
  emailHint?: string,
): Promise<ClaudeAccount> {
  return await invoke('claude_oauth_login_complete', {
    loginId,
    callbackOrCode,
    emailHint: emailHint?.trim() || null,
  });
}

export async function claudeOauthLoginCancel(loginId?: string): Promise<void> {
  return await invoke('claude_oauth_login_cancel', { loginId: loginId ?? null });
}

export async function exportClaudeAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_claude_accounts', { accountIds });
}

export async function refreshClaudeQuota(accountId: string): Promise<ClaudeAccount> {
  return await invoke('refresh_claude_quota', { accountId });
}

export async function refreshAllClaudeQuotas(): Promise<number> {
  return await invoke('refresh_all_claude_quotas');
}

export async function openClaudeVerificationWindow(accountId: string): Promise<void> {
  return await invoke('claude_open_verification_window', { accountId });
}

export async function updateClaudeAccountTags(
  accountId: string,
  tags: string[],
): Promise<ClaudeAccount> {
  return await invoke('update_claude_account_tags', { accountId, tags });
}

export async function updateClaudeAccountPlan(
  accountId: string,
  planType: string | null,
): Promise<ClaudeAccount> {
  return await invoke('update_claude_account_plan', {
    accountId,
    planType: planType?.trim() || null,
  });
}

export async function updateClaudeAccountNote(
  accountId: string,
  note: string,
): Promise<ClaudeAccount> {
  return await invoke('update_claude_account_note', {
    accountId,
    note: note.trim() || null,
  });
}

export async function getClaudeAccountsIndexPath(): Promise<string> {
  return await invoke('get_claude_accounts_index_path');
}

export async function switchClaudeAccount(accountId: string): Promise<string> {
  return await invoke('switch_claude_account', { accountId });
}

export async function getClaudeCliLaunchCommand(
  accountId: string,
  workingDir: string,
): Promise<ClaudeCliLaunchInfo> {
  return await invoke('claude_get_cli_launch_command', {
    accountId,
    workingDir,
  });
}

export async function executeClaudeCliLaunchCommand(
  accountId: string,
  workingDir: string,
  terminal?: string,
): Promise<string> {
  return await invoke('claude_execute_cli_launch_command', {
    accountId,
    workingDir,
    terminal: terminal?.trim() || null,
  });
}

export async function launchClaudeCli(
  accountId: string,
  workingDir: string,
  terminal?: string,
): Promise<string> {
  return await invoke('claude_launch_cli', {
    accountId,
    workingDir,
    terminal: terminal?.trim() || null,
  });
}
