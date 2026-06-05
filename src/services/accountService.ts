import { invoke } from '@tauri-apps/api/core';
import { Account, RefreshStats } from '../types/account';
import { AntigravityRuntimeTarget } from '../utils/antigravityRuntimeTarget';


export async function listAccounts(): Promise<Account[]> {
    return await invoke('list_accounts');
}

export async function addAccountWithToken(refreshToken: string): Promise<Account> {
    return await invoke('add_account', { refreshToken });
}

export async function addAccount(_email: string, refreshToken: string): Promise<Account> {
    return await addAccountWithToken(refreshToken);
}

export async function deleteAccount(accountId: string): Promise<void> {
    return await invoke('delete_account', { accountId });
}

export async function deleteAccounts(accountIds: string[]): Promise<void> {
    return await invoke('delete_accounts', { accountIds });
}

export async function reorderAccounts(accountIds: string[]): Promise<void> {
    return await invoke('reorder_accounts', { accountIds });
}

export async function getCurrentAccount(): Promise<Account | null> {
    return await invoke('get_current_account');
}

export async function setCurrentAccount(accountId: string): Promise<void> {
    return await invoke('set_current_account', { accountId });
}

export async function fetchAccountQuota(accountId: string): Promise<Account> {
    return await invoke('fetch_account_quota', { accountId });
}

export async function refreshAllQuotas(): Promise<RefreshStats> {
    return await invoke('refresh_all_quotas');
}

export async function startOAuthLogin(): Promise<Account> {
    return await invoke('start_oauth_login');
}

export async function prepareOAuthUrl(): Promise<string> {
    return await invoke('prepare_oauth_url');
}

export async function completeOAuthLogin(): Promise<Account> {
    return await invoke('complete_oauth_login');
}

export async function submitOAuthCallbackUrl(callbackUrl: string): Promise<void> {
    return await invoke('submit_oauth_callback_url', { callbackUrl });
}

export async function cancelOAuthLogin(): Promise<void> {
    return await invoke('cancel_oauth_login');
}

export async function openDataFolder(): Promise<void> {
    return await invoke('open_data_folder');
}

export async function switchAccount(
    accountId: string,
    runtimeTarget?: AntigravityRuntimeTarget,
): Promise<Account> {
    return await invoke('switch_account', { accountId, runtimeTarget });
}

export interface AntigravitySwitchHistoryItem {
    id: string;
    timestamp: number;
    accountId: string;
    targetEmail: string;
    triggerType?: string;
    triggerSource?: string;
    localOk: boolean;
    seamlessOk: boolean;
    success: boolean;
    localDurationMs: number;
    seamlessDurationMs?: number | null;
    totalDurationMs: number;
    errorStage?: string | null;
    errorCode?: string | null;
    errorMessage?: string | null;
    seamlessEffectiveMode?: string | null;
    seamlessFromEmail?: string | null;
  seamlessToEmail?: string | null;
  seamlessExecutionId?: string | null;
  seamlessFinishedAt?: string | null;
  autoSwitchReason?: AntigravityAutoSwitchReason | null;
}

export interface AntigravityAutoSwitchHitGroup {
  groupId: string;
  groupName: string;
  percentage: number;
}

export interface AntigravityAutoSwitchReason {
  rule: string;
  threshold: number;
  scopeMode: string;
  creditsEnabled?: boolean;
  creditsThreshold?: number | null;
  creditsTriggered?: boolean;
  currentCreditsRemaining?: number | null;
  selectedGroupIds: string[];
  selectedGroupNames: string[];
  hitGroups: AntigravityAutoSwitchHitGroup[];
  candidateCount: number;
  selectedPolicy: string;
}

export async function loadAntigravitySwitchHistory(): Promise<AntigravitySwitchHistoryItem[]> {
    return await invoke('load_antigravity_switch_history');
}

export async function clearAntigravitySwitchHistory(): Promise<void> {
    return await invoke('clear_antigravity_switch_history');
}

export async function updateAccountTags(accountId: string, tags: string[]): Promise<Account> {
    return await invoke('update_account_tags', { accountId, tags });
}

export async function updateAccountNotes(accountId: string, notes: string): Promise<Account> {
    return await invoke('update_account_notes', { accountId, notes });
}

export async function syncFromExtension(): Promise<number> {
    return await invoke('sync_from_extension');
}

export async function importFromOldTools(): Promise<Account[]> {
    return await invoke('import_from_old_tools');
}


export async function importFromLocal(): Promise<Account> {
    return await invoke('import_from_local');
}

export async function importFromJson(jsonContent: string): Promise<Account[]> {
    return await invoke('import_from_json', { jsonContent });
}

export interface FileImportResult {
    imported: Account[];
    failed: { email: string; error: string }[];
}

export async function importFromFiles(filePaths: string[]): Promise<FileImportResult> {
    return await invoke('import_from_files', { filePaths });
}

export async function exportAccounts(accountIds: string[]): Promise<string> {
    return await invoke('export_accounts', { accountIds });
}

export async function syncCurrentFromClient(): Promise<string | null> {
    return await invoke('sync_current_from_client');
}
