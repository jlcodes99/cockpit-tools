import { invoke } from '@tauri-apps/api/core';
import { WindsurfAccount } from '../types/windsurf';

export interface WindsurfOAuthLoginStartResponse {
  loginId: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

export interface WindsurfPasswordCredentialInput {
  email: string;
  password: string;
}

export interface WindsurfPasswordCredentialFailure {
  email: string;
  error: string;
}

export interface WindsurfPasswordBatchResult {
  accounts: WindsurfAccount[];
  success_count: number;
  failed_count: number;
  failures: WindsurfPasswordCredentialFailure[];
}

/** 列出所有 Windsurf 账号 */
export async function listWindsurfAccounts(): Promise<WindsurfAccount[]> {
  return await invoke('list_windsurf_accounts');
}

/** 删除 Windsurf 账号 */
export async function deleteWindsurfAccount(accountId: string): Promise<void> {
  return await invoke('delete_windsurf_account', { accountId });
}

/** 批量删除 Windsurf 账号 */
export async function deleteWindsurfAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_windsurf_accounts', { accountIds });
}

/** 从 JSON 字符串导入账号 */
export async function importWindsurfFromJson(jsonContent: string): Promise<WindsurfAccount[]> {
  return await invoke('import_windsurf_from_json', { jsonContent });
}

/** 从本机 Windsurf 客户端导入当前登录账号 */
export async function importWindsurfFromLocal(): Promise<WindsurfAccount[]> {
  return await invoke('import_windsurf_from_local');
}

/** 导出 Windsurf 账号 */
export async function exportWindsurfAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_windsurf_accounts', { accountIds });
}

/** 刷新单个账号 token/usage */
export async function refreshWindsurfToken(accountId: string): Promise<WindsurfAccount> {
  return await invoke('refresh_windsurf_token', { accountId });
}

/** 刷新全部账号 token/usage */
export async function refreshAllWindsurfTokens(): Promise<number> {
  return await invoke('refresh_all_windsurf_tokens');
}

/** Windsurf OAuth：开始登录（浏览器授权 + 本地回调） */
export async function startWindsurfOAuthLogin(): Promise<WindsurfOAuthLoginStartResponse> {
  return await invoke('windsurf_oauth_login_start');
}

/** Windsurf OAuth：完成登录（等待本地回调，直到成功/失败/超时） */
export async function completeWindsurfOAuthLogin(loginId: string): Promise<WindsurfAccount> {
  return await invoke('windsurf_oauth_login_complete', { loginId });
}

/** Windsurf OAuth：取消登录 */
export async function cancelWindsurfOAuthLogin(loginId?: string): Promise<void> {
  return await invoke('windsurf_oauth_login_cancel', { loginId: loginId ?? null });
}

/** Windsurf OAuth：手动提交回调链接 */
export async function submitWindsurfOAuthCallbackUrl(
  loginId: string,
  callbackUrl: string,
): Promise<void> {
  return await invoke('windsurf_oauth_submit_callback_url', { loginId, callbackUrl });
}

/** 通过 Windsurf access token 添加账号 */
export async function addWindsurfAccountWithToken(githubAccessToken: string): Promise<WindsurfAccount> {
  return await invoke('add_windsurf_account_with_token', { githubAccessToken });
}

/** 通过邮箱密码添加 Windsurf 账号 */
export async function addWindsurfAccountWithPassword(email: string, password: string): Promise<WindsurfAccount> {
  return await invoke('add_windsurf_account_with_password', { email, password });
}

/** 批量通过邮箱密码添加 Windsurf 账号 */
export async function addWindsurfAccountsWithPassword(
  credentials: WindsurfPasswordCredentialInput[],
): Promise<WindsurfPasswordBatchResult> {
  return await invoke('add_windsurf_accounts_with_password', { credentials });
}

export async function updateWindsurfAccountTags(accountId: string, tags: string[]): Promise<WindsurfAccount> {
  return await invoke('update_windsurf_account_tags', { accountId, tags });
}

export async function getWindsurfAccountsIndexPath(): Promise<string> {
  return await invoke('get_windsurf_accounts_index_path');
}

/** Inject a Windsurf account's token into VS Code's default instance */
export async function injectWindsurfToVSCode(accountId: string): Promise<string> {
  return await invoke('inject_windsurf_to_vscode', { accountId });
}
