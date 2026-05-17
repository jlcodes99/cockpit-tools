import { invoke } from '@tauri-apps/api/core';
import { DataTransferSelection, exportDataTransferJson } from './dataTransferService';
import { ALL_PLATFORM_IDS, PlatformId } from '../types/platform';

export type AutoBackupMode = 'full' | 'accounts' | 'config';
export type AutoBackupTrigger = 'auto' | 'manual';

export const AUTO_BACKUP_STATE_CHANGED_EVENT = 'auto-backup-state-changed';

const AUTO_BACKUP_INTERVAL_MS = 24 * 60 * 60 * 1000;

export interface AutoBackupSettings {
  enabled: boolean;
  include_accounts: boolean;
  include_config: boolean;
  retention_days: number;
  last_backup_at: string | null;
  directory_path: string;
}

export interface AutoBackupFileEntry {
  file_name: string;
  path: string;
  file_kind: 'json' | 'zip';
  size_bytes: number;
  modified_at_ms: number | null;
  archive_file_name?: string | null;
  archive_path?: string | null;
  archive_size_bytes?: number | null;
  platforms: AutoBackupPlatformEntry[];
}

export interface AutoBackupPlatformEntry {
  platform: PlatformId;
  account_count: number;
}

export interface ManagedBackupResult {
  file_name: string;
  path: string;
  executed_at: string;
  deleted_files: string[];
  selection: DataTransferSelection;
  trigger: AutoBackupTrigger;
}

function dispatchAutoBackupStateChanged() {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new Event(AUTO_BACKUP_STATE_CHANGED_EVENT));
}

function hasSelection(selection: DataTransferSelection): boolean {
  return selection.includeAccounts || selection.includeConfig;
}

function formatTimestampForFileName(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return [
    date.getFullYear(),
    pad(date.getMonth() + 1),
    pad(date.getDate()),
  ].join('-') + `_${pad(date.getHours())}-${pad(date.getMinutes())}-${pad(date.getSeconds())}`;
}

export function autoBackupModeToSelection(mode: AutoBackupMode): DataTransferSelection {
  if (mode === 'accounts') {
    return { includeAccounts: true, includeConfig: false };
  }
  if (mode === 'config') {
    return { includeAccounts: false, includeConfig: true };
  }
  return { includeAccounts: true, includeConfig: true };
}

export function selectionToAutoBackupMode(selection: DataTransferSelection): AutoBackupMode {
  if (selection.includeAccounts && selection.includeConfig) {
    return 'full';
  }
  if (selection.includeAccounts) {
    return 'accounts';
  }
  return 'config';
}

export function getSelectionFromAutoBackupSettings(
  settings: Pick<AutoBackupSettings, 'include_accounts' | 'include_config'>,
): DataTransferSelection {
  if (settings.include_accounts && settings.include_config) {
    return autoBackupModeToSelection('full');
  }
  if (settings.include_accounts) {
    return autoBackupModeToSelection('accounts');
  }
  if (settings.include_config) {
    return autoBackupModeToSelection('config');
  }
  return autoBackupModeToSelection('full');
}

function buildManagedBackupFileName(
  selection: DataTransferSelection,
  trigger: AutoBackupTrigger,
  date: Date,
): string {
  const mode = selectionToAutoBackupMode(selection);
  return `cockpit_${trigger}_backup_${mode}_${formatTimestampForFileName(date)}.json`;
}

function normalizeDate(value: string | null | undefined): Date | null {
  if (!value) return null;
  const parsed = new Date(value);
  return Number.isFinite(parsed.getTime()) ? parsed : null;
}

async function updateAutoBackupLastRunInternal(lastBackupAt: string | null): Promise<AutoBackupSettings> {
  return invoke<AutoBackupSettings>('update_auto_backup_last_run', {
    lastBackupAt,
  });
}

async function cleanupAutoBackupFilesInternal(retentionDays: number): Promise<string[]> {
  return invoke<string[]>('cleanup_auto_backup_files', {
    retentionDays,
  });
}

export async function getAutoBackupSettings(): Promise<AutoBackupSettings> {
  return invoke<AutoBackupSettings>('get_auto_backup_settings');
}

export async function saveAutoBackupSettings(params: {
  enabled: boolean;
  selection: DataTransferSelection;
  retentionDays: number;
}): Promise<AutoBackupSettings> {
  const next = await invoke<AutoBackupSettings>('save_auto_backup_settings', {
    enabled: params.enabled,
    includeAccounts: params.selection.includeAccounts,
    includeConfig: params.selection.includeConfig,
    retentionDays: params.retentionDays,
  });
  dispatchAutoBackupStateChanged();
  return next;
}

export async function updateAutoBackupLastRun(lastBackupAt: string | null): Promise<AutoBackupSettings> {
  const next = await updateAutoBackupLastRunInternal(lastBackupAt);
  dispatchAutoBackupStateChanged();
  return next;
}

export async function listAutoBackupFiles(): Promise<AutoBackupFileEntry[]> {
  return invoke<AutoBackupFileEntry[]>('list_auto_backup_files');
}

export async function readAutoBackupFile(fileName: string): Promise<string> {
  return invoke<string>('read_auto_backup_file', {
    fileName,
  });
}

export async function copyAutoBackupFile(fileName: string, targetPath: string): Promise<string> {
  return invoke<string>('copy_auto_backup_file', {
    fileName,
    targetPath,
  });
}

export async function deleteAutoBackupFile(fileName: string): Promise<void> {
  await invoke('delete_auto_backup_file', {
    fileName,
  });
  dispatchAutoBackupStateChanged();
}

export async function cleanupAutoBackupFiles(retentionDays: number): Promise<string[]> {
  const deleted = await cleanupAutoBackupFilesInternal(retentionDays);
  if (deleted.length > 0) {
    dispatchAutoBackupStateChanged();
  }
  return deleted;
}

export async function openAutoBackupDir(): Promise<void> {
  await invoke('open_auto_backup_dir');
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function resolveBackupPlatformPayload(jsonContent: string, platform: PlatformId): unknown {
  const parsed = JSON.parse(jsonContent) as unknown;
  if (!isRecord(parsed)) {
    throw new Error('invalid_backup_json');
  }
  const accountBundle = isRecord(parsed.accounts) ? parsed.accounts : parsed;
  if (!isRecord(accountBundle.platforms)) {
    throw new Error('backup_accounts_missing');
  }
  const payload = accountBundle.platforms[platform];
  if (!isRecord(payload)) {
    throw new Error('backup_platform_missing');
  }
  return payload.exported_data ?? payload.data ?? payload.accounts ?? [];
}

export function extractAutoBackupPlatformJson(jsonContent: string, platform: PlatformId): string {
  const payload = resolveBackupPlatformPayload(jsonContent, platform);
  return JSON.stringify(payload, null, 2);
}

export function normalizeAutoBackupPlatforms(
  platforms: AutoBackupPlatformEntry[] | undefined,
): AutoBackupPlatformEntry[] {
  const countByPlatform = new Map<PlatformId, number>();
  for (const item of platforms ?? []) {
    if (!ALL_PLATFORM_IDS.includes(item.platform)) continue;
    const count = Number.isFinite(item.account_count)
      ? Math.max(0, Math.floor(item.account_count))
      : 0;
    if (count <= 0) continue;
    countByPlatform.set(item.platform, (countByPlatform.get(item.platform) ?? 0) + count);
  }
  return ALL_PLATFORM_IDS
    .filter((platform) => countByPlatform.has(platform))
    .map((platform) => ({
      platform,
      account_count: countByPlatform.get(platform) ?? 0,
    }));
}

export function isAutoBackupDue(settings: AutoBackupSettings, now = new Date()): boolean {
  if (!settings.enabled) return false;
  const selection = getSelectionFromAutoBackupSettings(settings);
  if (!hasSelection(selection)) return false;
  const lastBackupAt = normalizeDate(settings.last_backup_at);
  if (!lastBackupAt) return true;
  return now.getTime() - lastBackupAt.getTime() >= AUTO_BACKUP_INTERVAL_MS;
}

export async function createManagedBackup(params: {
  trigger: AutoBackupTrigger;
  selection: DataTransferSelection;
  retentionDays: number;
  markAsLastRun?: boolean;
}): Promise<ManagedBackupResult> {
  if (!hasSelection(params.selection)) {
    throw new Error('transfer_selection_required');
  }

  const executedAt = new Date();
  const content = await exportDataTransferJson(params.selection);
  const fileName = buildManagedBackupFileName(params.selection, params.trigger, executedAt);
  const path = await invoke<string>('write_auto_backup_file', {
    fileName,
    content,
  });
  const deletedFiles = await cleanupAutoBackupFilesInternal(params.retentionDays);

  if (params.markAsLastRun !== false) {
    await updateAutoBackupLastRunInternal(executedAt.toISOString());
  }

  dispatchAutoBackupStateChanged();

  return {
    file_name: fileName,
    path,
    executed_at: executedAt.toISOString(),
    deleted_files: deletedFiles,
    selection: params.selection,
    trigger: params.trigger,
  };
}

export async function runAutoBackupCycle(): Promise<ManagedBackupResult | null> {
  const settings = await getAutoBackupSettings();
  const selection = getSelectionFromAutoBackupSettings(settings);

  if (!settings.enabled || !hasSelection(selection) || !isAutoBackupDue(settings)) {
    return null;
  }

  return createManagedBackup({
    trigger: 'auto',
    selection,
    retentionDays: settings.retention_days,
    markAsLastRun: true,
  });
}
