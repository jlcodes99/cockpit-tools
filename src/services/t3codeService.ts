import { invoke } from '@tauri-apps/api/core';
import type {
  T3CodeAccount,
  T3CodeDriverKind,
  T3CodeNativeSettings,
  T3CodeProviderRuntimeView,
  T3CodeProviderSwitchView,
  T3CodeProviderWorkspaceView,
  T3CodeQuotaWindowView,
} from '../types/t3code';

export async function listT3codeAccounts(): Promise<T3CodeAccount[]> {
  return await invoke('list_t3code_accounts');
}

export async function deleteT3codeAccount(accountId: string): Promise<void> {
  await invoke('delete_t3code_account', { accountId });
}

export async function deleteT3codeAccounts(accountIds: string[]): Promise<void> {
  await invoke('delete_t3code_accounts', { accountIds });
}

export async function importT3codeFromJson(jsonContent: string): Promise<T3CodeAccount[]> {
  return await invoke('import_t3code_from_json', { jsonContent });
}

export async function importT3codeFromFile(path: string): Promise<T3CodeAccount[]> {
  return await invoke('import_t3code_from_file', { path });
}

export async function importT3codeFromLocal(): Promise<T3CodeAccount[]> {
  return await invoke('import_t3code_from_local');
}

export async function importT3codeToken(
  token: string,
  accountName?: string,
): Promise<T3CodeAccount> {
  return await invoke('import_t3code_token', {
    token,
    accountName: accountName ?? null,
  });
}

export async function exportT3codeAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_t3code_accounts', { accountIds });
}

export async function refreshT3codeAccount(accountId: string): Promise<T3CodeAccount> {
  return await invoke('refresh_t3code_account', { accountId });
}

export async function refreshAllT3codeAccounts(): Promise<number> {
  return await invoke('refresh_all_t3code_accounts');
}

export async function injectT3codeAccount(accountId: string): Promise<string> {
  return await invoke('inject_t3code_account', { accountId });
}

export async function launchT3codeCurrent(): Promise<void> {
  await invoke('t3code_launch_current');
}

export async function updateT3codeAccountTags(
  accountId: string,
  tags: string[],
): Promise<T3CodeAccount> {
  return await invoke('update_t3code_account_tags', { accountId, tags });
}

export async function getT3codeCurrentAccountId(): Promise<string | null> {
  return await invoke('get_t3code_current_account_id');
}

export async function getT3codeAccountsIndexPath(): Promise<string> {
  return await invoke('get_t3code_accounts_index_path');
}

export function getT3codeProviderWorkspace(
  t3InstanceId: string | null,
): Promise<T3CodeProviderWorkspaceView> {
  return invoke('get_t3code_provider_workspace', { t3InstanceId });
}

export function getT3codeProviderRuntimeStatus(t3InstanceId: string): Promise<T3CodeProviderRuntimeView[]> {
  return invoke('get_t3code_provider_runtime_status', { t3InstanceId });
}

export function getT3codeApiServiceQuota(): Promise<T3CodeQuotaWindowView[] | null> {
  return invoke('get_t3code_api_service_quota');
}

export function syncT3codeProviderLaunchPaths(t3InstanceId: string): Promise<void> {
  return invoke('t3code_sync_provider_launch_paths', { t3InstanceId });
}

export function installT3codeAntigravityRuntime(instanceId: string): Promise<void> {
  return invoke('t3code_install_antigravity_runtime', { instanceId });
}

export function bindT3codeProviderInstanceAccount(params: {
  t3InstanceId: string | null;
  providerInstanceId: string;
  driver: T3CodeDriverKind;
  cockpitAccountId: string;
}): Promise<T3CodeProviderWorkspaceView> {
  return invoke('bind_t3code_provider_instance_account', params);
}

export function switchT3codeProviderInstanceAccount(params: {
  t3InstanceId: string | null;
  providerInstanceId: string;
  driver: T3CodeDriverKind;
  cockpitAccountId: string;
}): Promise<T3CodeProviderSwitchView> {
  return invoke('switch_t3code_provider_instance_account', params);
}

export function createT3codeProviderInstance(params: {
  t3InstanceId: string | null;
  instanceId: string;
  driver: T3CodeDriverKind;
  displayName: string;
  bindAccountId?: string | null;
  enabled?: boolean;
}): Promise<T3CodeProviderWorkspaceView> {
  return invoke('upsert_t3code_provider_instance', {
    ...params,
    bindAccountId: params.bindAccountId?.trim() ? params.bindAccountId.trim() : null,
    enabled: params.enabled ?? true,
    accentColor: null,
  });
}

export function updateT3codeProviderInstance(params: {
  t3InstanceId: string | null;
  instanceId: string;
  driver: T3CodeDriverKind;
  displayName?: string;
  enabled: boolean;
}): Promise<T3CodeProviderWorkspaceView> {
  return invoke('upsert_t3code_provider_instance', {
    t3InstanceId: params.t3InstanceId,
    instanceId: params.instanceId,
    driver: params.driver,
    displayName: params.displayName ?? null,
    enabled: params.enabled,
    accentColor: null,
    bindAccountId: null,
  });
}

export function removeT3codeProviderInstance(params: {
  t3InstanceId: string | null;
  instanceId: string;
  cockpitAccountId?: string | null;
}): Promise<T3CodeProviderSwitchView> {
  return invoke('remove_t3code_provider_instance', params);
}

export function promoteT3codeProviderAccount(params: {
  t3InstanceId: string | null;
  providerInstanceId: string;
  cockpitAccountId: string;
}): Promise<T3CodeProviderSwitchView> {
  return invoke('promote_t3code_provider_account', params);
}

export function refreshT3codeProviderAccountQuota(params: {
  t3InstanceId: string | null;
  providerInstanceId: string;
}): Promise<T3CodeProviderWorkspaceView> {
  return invoke('refresh_t3code_provider_account_quota', params);
}


export function getT3codeNativeSettings(): Promise<T3CodeNativeSettings> {
  return invoke('get_t3code_native_settings');
}

export function updateT3codeNativeSettings(
  field: 'language' | 'notificationMode' | 'defaultModelSelection',
  value: string | null | { instanceId: string; model: string; effort?: string },
): Promise<T3CodeNativeSettings> {
  return invoke('update_t3code_native_settings', { field, value });
}
