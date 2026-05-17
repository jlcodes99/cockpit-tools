import { invoke } from '@tauri-apps/api/core';
import {
  CodexAccount,
  CodexApiProviderMode,
  CodexAppSpeed,
  CodexAppSpeedConfig,
  CodexQuickConfig,
  CodexQuota,
} from '../types/codex';

export interface CodexOAuthLoginStartResponse {
  loginId: string;
  authUrl: string;
}

/** 列出所有 Codex 账号 */
export async function listCodexAccounts(): Promise<CodexAccount[]> {
  return await invoke('list_codex_accounts');
}

/** 获取当前激活的 Codex 账号 */
export async function getCurrentCodexAccount(): Promise<CodexAccount | null> {
  return await invoke('get_current_codex_account');
}

/** 获取当前 Codex config.toml 路径 */
export async function getCodexConfigTomlPath(): Promise<string> {
  return await invoke('get_codex_config_toml_path');
}

/** 打开当前 Codex config.toml */
export async function openCodexConfigToml(): Promise<void> {
  return await invoke('open_codex_config_toml');
}

/** 获取 Codex config.toml 快捷配置 */
export async function getCodexQuickConfig(): Promise<CodexQuickConfig> {
  return await invoke('get_codex_quick_config');
}

/** 保存 Codex config.toml 快捷配置 */
export async function saveCodexQuickConfig(
  modelContextWindow?: number,
  autoCompactTokenLimit?: number,
): Promise<CodexQuickConfig> {
  return await invoke('save_codex_quick_config', {
    modelContextWindow: modelContextWindow ?? null,
    autoCompactTokenLimit: autoCompactTokenLimit ?? null,
  });
}

/** 获取 Codex 官方 App 速度配置 */
export async function getCodexAppSpeedConfig(): Promise<CodexAppSpeedConfig> {
  return await invoke('get_codex_app_speed_config');
}

/** 保存 Codex 官方 App 速度配置 */
export async function saveCodexAppSpeed(speed: CodexAppSpeed): Promise<CodexAppSpeedConfig> {
  return await invoke('save_codex_app_speed', { speed });
}

export async function getCodexApiServiceAppSpeedConfig(): Promise<CodexAppSpeedConfig> {
  return await invoke('get_codex_api_service_app_speed_config');
}

export async function saveCodexApiServiceAppSpeed(speed: CodexAppSpeed): Promise<CodexAppSpeedConfig> {
  return await invoke('save_codex_api_service_app_speed', { speed });
}

export async function updateCodexAccountAppSpeed(
  accountId: string,
  speed: CodexAppSpeed,
): Promise<CodexAccount> {
  return await invoke('update_codex_account_app_speed', { accountId, speed });
}

/** 刷新 Codex 账号资料（团队名/结构） */
export async function refreshCodexAccountProfile(accountId: string): Promise<CodexAccount> {
  return await invoke('refresh_codex_account_profile', { accountId });
}

/** 切换 Codex 账号 */
export async function switchCodexAccount(accountId: string): Promise<CodexAccount> {
  return await invoke('switch_codex_account', { accountId });
}

/** 删除 Codex 账号 */
export async function deleteCodexAccount(accountId: string): Promise<void> {
  return await invoke('delete_codex_account', { accountId });
}

/** 批量删除 Codex 账号 */
export async function deleteCodexAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_codex_accounts', { accountIds });
}

/** 从本地 auth.json 导入账号 */
export async function importCodexFromLocal(): Promise<CodexAccount> {
  return await invoke('import_codex_from_local');
}

/** 从 JSON 字符串导入账号 */
export async function importCodexFromJson(jsonContent: string): Promise<CodexAccount[]> {
  return await invoke('import_codex_from_json', { jsonContent });
}

/** 导出 Codex 账号 */
export async function exportCodexAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_codex_accounts', { accountIds });
}

export interface CodexFileImportResult {
  imported: CodexAccount[];
  failed: { email: string; error: string }[];
}

/** 从本地文件导入 Codex 账号 */
export async function importCodexFromFiles(filePaths: string[]): Promise<CodexFileImportResult> {
  return await invoke('import_codex_from_files', { filePaths });
}

/** 刷新单个账号配额 */
export async function refreshCodexQuota(accountId: string): Promise<CodexQuota> {
  return await invoke('refresh_codex_quota', { accountId });
}

/** 刷新所有账号配额 */
export async function refreshAllCodexQuotas(): Promise<number> {
  return await invoke('refresh_all_codex_quotas');
}

/** 新 OAuth 流程：开始登录 */
export async function startCodexOAuthLogin(): Promise<CodexOAuthLoginStartResponse> {
  return await invoke('codex_oauth_login_start');
}

/** 新 OAuth 流程：完成登录 */
export async function completeCodexOAuthLogin(loginId: string): Promise<CodexAccount> {
  return await invoke('codex_oauth_login_completed', { loginId });
}

/** 新 OAuth 流程：取消登录 */
export async function cancelCodexOAuthLogin(loginId?: string): Promise<void> {
  return await invoke('codex_oauth_login_cancel', { loginId: loginId ?? null });
}

/** 新 OAuth 流程：手动提交回调链接 */
export async function submitCodexOAuthCallbackUrl(
  loginId: string,
  callbackUrl: string,
): Promise<void> {
  return await invoke('codex_oauth_submit_callback_url', { loginId, callbackUrl });
}

/** 通过 Token 添加账号 */
export async function addCodexAccountWithToken(
  idToken: string,
  accessToken: string,
  refreshToken?: string
): Promise<CodexAccount> {
  return await invoke('add_codex_account_with_token', {
    idToken,
    accessToken,
    refreshToken: refreshToken ?? null,
  });
}

/** 通过 API Key 添加账号 */
export async function addCodexAccountWithApiKey(
  apiKey: string,
  apiBaseUrl?: string,
  apiProviderMode?: CodexApiProviderMode,
  apiProviderId?: string,
  apiProviderName?: string,
): Promise<CodexAccount> {
  return await invoke('add_codex_account_with_api_key', {
    apiKey,
    apiBaseUrl: apiBaseUrl ?? null,
    apiProviderMode: apiProviderMode ?? null,
    apiProviderId: apiProviderId ?? null,
    apiProviderName: apiProviderName ?? null,
  });
}

export async function updateCodexAccountName(accountId: string, name: string): Promise<CodexAccount> {
  return await invoke('update_codex_account_name', { accountId, name });
}

export async function updateCodexApiKeyCredentials(
  accountId: string,
  apiKey: string,
  apiBaseUrl?: string,
  apiProviderMode?: CodexApiProviderMode,
  apiProviderId?: string,
  apiProviderName?: string,
): Promise<CodexAccount> {
  return await invoke('update_codex_api_key_credentials', {
    accountId,
    apiKey,
    apiBaseUrl: apiBaseUrl ?? null,
    apiProviderMode: apiProviderMode ?? null,
    apiProviderId: apiProviderId ?? null,
    apiProviderName: apiProviderName ?? null,
  });
}

export async function updateCodexApiKeyBoundOAuthAccount(
  accountId: string,
  boundOauthAccountId: string | null,
): Promise<CodexAccount> {
  return await invoke('update_codex_api_key_bound_oauth_account', {
    accountId,
    boundOauthAccountId,
  });
}

/** 检查 Codex OAuth 端口是否被占用 */
export async function isCodexOAuthPortInUse(): Promise<boolean> {
  return await invoke('is_codex_oauth_port_in_use');
}

/** 关闭占用 Codex OAuth 端口的进程 */
export async function closeCodexOAuthPort(): Promise<number> {
  return await invoke('close_codex_oauth_port');
}

export async function updateCodexAccountTags(accountId: string, tags: string[]): Promise<CodexAccount> {
  return await invoke('update_codex_account_tags', { accountId, tags });
}

export async function updateCodexAccountNote(accountId: string, note: string): Promise<CodexAccount> {
  return await invoke('update_codex_account_note', { accountId, note });
}
