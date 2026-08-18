import { invoke } from '@tauri-apps/api/core';
import {
  CodexAccount,
  CodexAccountNoteUpdate,
  CodexApiModelMapping,
  CodexApiProviderMode,
  CodexAppSpeed,
  CodexAppSpeedConfig,
  CodexFingerprintMode,
  CodexBatchDeleteJobStatus,
  CodexProviderWireApi,
  CodexQuickConfig,
  CodexExperimentalModelDefinition,
  CodexQuota,
  CodexResetCreditsSnapshot,
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
  experimentalModelCatalogEnabled?: boolean,
  experimentalModelCatalogModels?: CodexExperimentalModelDefinition[],
  experimentalModelCatalogDefaultModelId?: string | null,
): Promise<CodexQuickConfig> {
  return await invoke('save_codex_quick_config', {
    modelContextWindow: modelContextWindow ?? null,
    autoCompactTokenLimit: autoCompactTokenLimit ?? null,
    experimentalModelCatalogEnabled: experimentalModelCatalogEnabled ?? null,
    experimentalModelCatalogModels: experimentalModelCatalogModels ?? null,
    experimentalModelCatalogDefaultModelId: experimentalModelCatalogDefaultModelId ?? null,
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
export async function switchCodexAccount(
  accountId: string,
): Promise<CodexAccount> {
  const startedAt = performance.now();
  console.info('[Codex Switch][Service] invoke switch_codex_account started', {
    accountId,
  });
  try {
    return await invoke('switch_codex_account', {
      accountId,
      autoRepairMode: null,
    });
  } finally {
    console.info('[Codex Switch][Service] invoke switch_codex_account finished', {
      accountId,
      elapsedMs: Math.round(performance.now() - startedAt),
    });
  }
}

/** 删除 Codex 账号 */
export async function deleteCodexAccount(accountId: string): Promise<void> {
  return await invoke('delete_codex_account', { accountId });
}

/** 批量删除 Codex 账号 */
export async function deleteCodexAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_codex_accounts', { accountIds });
}

export async function startCodexBatchDelete(
  accountIds: string[],
): Promise<CodexBatchDeleteJobStatus> {
  return await invoke('start_codex_batch_delete', { accountIds });
}

export async function getCodexBatchDelete(
  jobId: string,
): Promise<CodexBatchDeleteJobStatus> {
  return await invoke('get_codex_batch_delete', { jobId });
}

export async function resumeCodexBatchDelete(
  jobId: string,
): Promise<CodexBatchDeleteJobStatus> {
  return await invoke('resume_codex_batch_delete', { jobId });
}

export async function pauseCodexBatchDelete(
  jobId: string,
): Promise<CodexBatchDeleteJobStatus> {
  return await invoke('pause_codex_batch_delete', { jobId });
}

export async function retryFailedCodexBatchDelete(
  jobId: string,
): Promise<CodexBatchDeleteJobStatus> {
  return await invoke('retry_failed_codex_batch_delete', { jobId });
}

export async function clearCodexBatchDelete(jobId: string): Promise<void> {
  return await invoke('clear_codex_batch_delete', { jobId });
}

/** 从本地 auth.json 导入账号 */
/** 导入 named Codex access token（personal access token / at-*）账号 */
export async function importCodexAccessTokenAccount(
  name: string,
  accessToken: string,
): Promise<CodexAccount> {
  return await invoke('import_codex_access_token_account', {
    name,
    accessToken,
  });
}

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

export interface CodexBatchImportStartResult {
  sessionId: string;
}

export interface CodexBatchImportProgress {
  sessionId: string;
  phase: string;
  checkQuota: boolean;
  current: number;
  total: number;
  success: number;
  failed: number;
  quotaFailed: number;
  existing: number;
  currentLabel?: string | null;
}

export interface CodexBatchImportItem {
  itemId: string;
  source: string;
  label: string;
  accountId?: string | null;
  email?: string | null;
  accountType: string;
  provider?: string | null;
  quotaStatus: string;
  quotaError?: string | null;
  status: string;
  error?: string | null;
  defaultSelected: boolean;
  selectable: boolean;
  existing: boolean;
}

export interface CodexBatchImportPreview {
  sessionId: string;
  status: string;
  checkQuota: boolean;
  total: number;
  items: CodexBatchImportItem[];
}

export interface CodexBatchImportConfirmResult {
  imported: CodexAccount[];
  failed: { email: string; error: string }[];
  cancelled: boolean;
  processed: number;
  total: number;
}

export async function startCodexBatchImportFromFiles(
  filePaths: string[],
  checkQuota = false,
): Promise<CodexBatchImportStartResult> {
  return await invoke('start_codex_batch_import_from_files', { filePaths, checkQuota });
}

export async function cancelCodexBatchImport(sessionId: string): Promise<void> {
  return await invoke('cancel_codex_batch_import', { sessionId });
}

export async function resumeCodexBatchImport(sessionId: string): Promise<void> {
  return await invoke('resume_codex_batch_import', { sessionId });
}

export async function getCodexBatchImportPreview(
  sessionId: string,
): Promise<CodexBatchImportPreview> {
  return await invoke('get_codex_batch_import_preview', { sessionId });
}

export async function confirmCodexBatchImport(
  sessionId: string,
  itemIds: string[],
): Promise<CodexBatchImportConfirmResult> {
  return await invoke('confirm_codex_batch_import', { sessionId, itemIds });
}

/** 刷新单个账号配额 */
export async function refreshCodexQuota(accountId: string): Promise<CodexQuota> {
  return await invoke('refresh_codex_quota', { accountId });
}

/** 获取 Codex 主动重置次数明细 */
export async function getCodexResetCredits(
  accountId: string,
): Promise<CodexResetCreditsSnapshot> {
  return await invoke('get_codex_reset_credits', { accountId });
}

/** 消耗一次 Codex 主动重置次数 */
export async function consumeCodexResetCredit(accountId: string): Promise<void> {
  return await invoke('consume_codex_reset_credit', { accountId });
}

/** 强制刷新单个账号的订阅信息 */
export async function refreshCodexSubscriptionInfo(accountId: string): Promise<CodexAccount> {
  return await invoke('refresh_codex_subscription_info', { accountId });
}

/** 刷新所有账号配额 */
export async function refreshAllCodexQuotas(): Promise<number> {
  return await invoke('refresh_all_codex_quotas');
}

/** 按 ID 列表限流并发刷新配额（分组/本地访问批量）；后端统一限流并只做一次 tray 更新 */
export async function refreshCodexQuotasBatch(
  accountIds: string[],
  options?: { respectGroupQuotaRefresh?: boolean },
): Promise<number> {
  return await invoke('refresh_codex_quotas_batch', {
    accountIds,
    // 缺省 true：遵守分组「额度刷新」开关；显式刷新分组时传 false
    respectGroupQuotaRefresh: options?.respectGroupQuotaRefresh ?? true,
  });
}

/** 新 OAuth 流程：开始登录 */
export async function startCodexOAuthLogin(): Promise<CodexOAuthLoginStartResponse> {
  return await invoke('codex_oauth_login_start');
}

/** 在内置无痕 WebView 中打开当前 Codex OAuth 授权地址 */
export async function openCodexOAuthIncognitoWindow(authUrl: string): Promise<void> {
  await invoke('codex_oauth_open_incognito_window', { authUrl });
}

/** 新 OAuth 流程：完成登录 */
export async function completeCodexOAuthLogin(
  loginId: string,
  reauthAccountId?: string | null
): Promise<CodexAccount> {
  return await invoke('codex_oauth_login_completed', {
    loginId,
    reauthAccountId: reauthAccountId ?? null,
  });
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
  apiModelCatalog?: string[],
  apiSupportsVision?: boolean,
  apiModelVisionSupport?: Record<string, boolean>,
  apiVisionRoutingModel?: string,
  accountName?: string,
  apiWireApi?: CodexProviderWireApi,
  apiSupportsWebsockets?: boolean,
  apiSyncModelCatalogToCodex?: boolean,
  apiModelContextWindows?: Record<string, number>,
): Promise<CodexAccount> {
  return await invoke('add_codex_account_with_api_key', {
    apiKey,
    apiBaseUrl: apiBaseUrl ?? null,
    apiProviderMode: apiProviderMode ?? null,
    apiProviderId: apiProviderId ?? null,
    apiProviderName: apiProviderName ?? null,
    apiModelCatalog: apiModelCatalog ?? null,
    apiSyncModelCatalogToCodex: apiSyncModelCatalogToCodex ?? null,
    apiWireApi: apiWireApi ?? null,
    apiSupportsWebsockets: apiSupportsWebsockets ?? false,
    apiSupportsVision: apiSupportsVision ?? false,
    apiModelVisionSupport: apiModelVisionSupport ?? {},
    apiVisionRoutingModel: apiVisionRoutingModel ?? null,
    accountName: accountName ?? null,
    apiModelContextWindows: apiModelContextWindows ?? null,
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
  apiModelCatalog?: string[],
  apiSupportsVision?: boolean,
  apiModelVisionSupport?: Record<string, boolean>,
  apiVisionRoutingModel?: string,
  apiWireApi?: CodexProviderWireApi,
  apiSupportsWebsockets?: boolean,
  apiSyncModelCatalogToCodex?: boolean,
  accountName?: string,
  apiModelContextWindows?: Record<string, number>,
): Promise<CodexAccount> {
  return await invoke('update_codex_api_key_credentials', {
    accountId,
    apiKey,
    apiBaseUrl: apiBaseUrl ?? null,
    apiProviderMode: apiProviderMode ?? null,
    apiProviderId: apiProviderId ?? null,
    apiProviderName: apiProviderName ?? null,
    apiModelCatalog: apiModelCatalog ?? null,
    apiSyncModelCatalogToCodex: apiSyncModelCatalogToCodex ?? null,
    apiWireApi: apiWireApi ?? null,
    apiSupportsWebsockets: apiSupportsWebsockets ?? false,
    apiSupportsVision: apiSupportsVision ?? false,
    apiModelVisionSupport: apiModelVisionSupport ?? {},
    apiVisionRoutingModel: apiVisionRoutingModel ?? null,
    accountName: accountName ?? null,
    apiModelContextWindows: apiModelContextWindows ?? null,
  });
}

export async function syncCodexApiKeyProviderAccounts(input: {
  accountIds: string[];
  apiBaseUrl: string;
  apiProviderMode: CodexApiProviderMode;
  apiProviderId: string;
  apiProviderName: string;
  apiModelCatalog?: string[];
  apiModelContextWindows?: Record<string, number>;
  apiWireApi: CodexProviderWireApi;
  apiSupportsWebsockets: boolean;
  apiSupportsVision: boolean;
  apiModelVisionSupport: Record<string, boolean>;
  apiVisionRoutingModel?: string;
}): Promise<number> {
  return await invoke('sync_codex_api_key_provider_accounts', {
    accountIds: input.accountIds,
    apiBaseUrl: input.apiBaseUrl,
    apiProviderMode: input.apiProviderMode,
    apiProviderId: input.apiProviderId,
    apiProviderName: input.apiProviderName,
    apiModelCatalog: input.apiModelCatalog ?? null,
    apiModelContextWindows: input.apiModelContextWindows ?? null,
    apiWireApi: input.apiWireApi,
    apiSupportsWebsockets: input.apiSupportsWebsockets,
    apiSupportsVision: input.apiSupportsVision,
    apiModelVisionSupport: input.apiModelVisionSupport,
    apiVisionRoutingModel: input.apiVisionRoutingModel ?? null,
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

export async function updateCodexAccountsFingerprintMode(
  accountIds: string[],
  mode: CodexFingerprintMode,
): Promise<CodexAccount[]> {
  return await invoke('update_codex_accounts_fingerprint_mode', { accountIds, mode });
}

export async function updateCodexAccountClientPolicy(
  accountId: string,
  codexCliOnly: boolean,
  allowAppServer: boolean,
): Promise<CodexAccount> {
  return await invoke('update_codex_account_client_policy', {
    accountId,
    codexCliOnly,
    allowAppServer,
  });
}

export async function updateCodexAccountInstanceAccess(
  accountId: string,
  accessMode?: string | null,
  startupModel?: string | null,
): Promise<CodexAccount> {
  return await invoke("update_codex_account_instance_access", {
    accountId,
    accessMode: accessMode ?? null,
    startupModel: startupModel ?? null,
  });
}

export async function updateCodexAccountApiModelMappings(
  accountId: string,
  mappings: CodexApiModelMapping[],
  apiModelContextWindows?: Record<string, number>,
): Promise<CodexAccount> {
  return await invoke('update_codex_account_api_model_mappings', {
    accountId,
    mappings,
    apiModelContextWindows: apiModelContextWindows ?? null,
  });
}

export async function updateCodexAccountNote(
  accountId: string,
  update: string | CodexAccountNoteUpdate,
): Promise<CodexAccount> {
  const payload = typeof update === 'string' ? { note: update } : update;
  return await invoke('update_codex_account_note', { accountId, ...payload });
}

export async function createPendingCodexOAuthAccount(
  email: string,
  update: CodexAccountNoteUpdate,
): Promise<CodexAccount> {
  return await invoke('create_pending_codex_oauth_account', { email, ...update });
}

export interface CodexMailPreviewFetchResult {
  status: number;
  contentType?: string | null;
  body: string;
  truncated: boolean;
}

export async function fetchCodexAccountNoteMailUrl(
  mailUrl: string,
): Promise<CodexMailPreviewFetchResult> {
  return await invoke('fetch_codex_account_note_mail_url', { mailUrl });
}
