import { invoke } from '@tauri-apps/api/core';

export interface ManagedLogFile {
  log_file_path: string;
  log_file_name: string;
  file_size: number;
  modified_at_ms: number | null;
}

export interface LogSnapshot {
  log_dir_path: string;
  log_file_path: string;
  log_file_name: string;
  content: string | null;
  content_revision: string | null;
  line_limit: number;
  file_size: number;
  modified_at_ms: number | null;
  available_files: ManagedLogFile[];
}

export async function getLogSnapshot(
  fileName?: string,
  lineLimit?: number,
  knownContentRevision?: string,
): Promise<LogSnapshot> {
  return await invoke('logs_get_snapshot', {
    fileName: fileName ?? null,
    lineLimit,
    knownContentRevision,
  });
}

export async function openLogDirectory(): Promise<void> {
  return await invoke('logs_open_log_directory');
}
