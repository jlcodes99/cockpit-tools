import { invoke } from '@tauri-apps/api/core';

export const WEBDAV_SYNC_STATE_CHANGED_EVENT = 'webdav-sync-state-changed';

export interface WebdavSyncSettings {
  enabled: boolean;
  url: string;
  username: string;
  has_password: boolean;
  remote_dir: string;
  last_upload_at: string | null;
  last_upload_file_name: string | null;
  last_download_at: string | null;
  last_download_file_name: string | null;
}

export interface WebdavBackupFileEntry {
  file_name: string;
  file_kind: 'json' | 'zip';
  size_bytes: number;
  modified_at: string | null;
}

export interface WebdavTestResult {
  ok: boolean;
  message: string;
}

export interface WebdavUploadResult {
  uploaded_files: WebdavBackupFileEntry[];
  deleted_files: string[];
  uploaded_at: string;
  remote_dir: string;
}

export interface SaveWebdavSyncSettingsParams {
  enabled: boolean;
  url: string;
  username: string;
  password?: string | null;
  clearPassword?: boolean;
  remoteDir: string;
}

export function dispatchWebdavSyncStateChanged() {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new Event(WEBDAV_SYNC_STATE_CHANGED_EVENT));
}

export async function getWebdavSyncSettings(): Promise<WebdavSyncSettings> {
  return invoke<WebdavSyncSettings>('get_webdav_sync_settings');
}

export async function saveWebdavSyncSettings(
  params: SaveWebdavSyncSettingsParams,
): Promise<WebdavSyncSettings> {
  const next = await invoke<WebdavSyncSettings>('save_webdav_sync_settings', {
    enabled: params.enabled,
    url: params.url,
    username: params.username,
    password: params.password ?? null,
    clearPassword: params.clearPassword ?? false,
    remoteDir: params.remoteDir,
  });
  dispatchWebdavSyncStateChanged();
  return next;
}

export async function testWebdavSyncConnection(
  params: Omit<SaveWebdavSyncSettingsParams, 'enabled'>,
): Promise<WebdavTestResult> {
  return invoke<WebdavTestResult>('test_webdav_sync_connection', {
    url: params.url,
    username: params.username,
    password: params.password ?? null,
    clearPassword: params.clearPassword ?? false,
    remoteDir: params.remoteDir,
  });
}

export async function uploadAutoBackupToWebdav(fileName: string): Promise<WebdavUploadResult> {
  const result = await invoke<WebdavUploadResult>('upload_auto_backup_to_webdav', {
    fileName,
  });
  dispatchWebdavSyncStateChanged();
  return result;
}

export async function listWebdavBackupFiles(): Promise<WebdavBackupFileEntry[]> {
  return invoke<WebdavBackupFileEntry[]>('list_webdav_backup_files');
}

export async function readWebdavBackupFile(fileName: string): Promise<string> {
  const content = await invoke<string>('read_webdav_backup_file', {
    fileName,
  });
  dispatchWebdavSyncStateChanged();
  return content;
}

export async function deleteWebdavBackupFile(fileName: string): Promise<void> {
  await invoke('delete_webdav_backup_file', {
    fileName,
  });
  dispatchWebdavSyncStateChanged();
}
