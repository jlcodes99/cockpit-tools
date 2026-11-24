import type { LocalHistoryRead } from '../hooks/useLocalHistory';
/**
 * WorkBuddy AI（国际版）自动活跃打卡服务
 *
 * 由 Rust 持久化配置并负责实际后台调度，前端仅缓存配置与转发命令。
 */

import { invoke } from '@tauri-apps/api/core';

export interface WorkbuddyAiAccountScheduleState {
  scheduledDate: string; // "YYYY-MM-DD"
  scheduledMinute: number; // Minutes from midnight (0..1439)
  lastCheckedDate?: string; // "YYYY-MM-DD" when checked in
}

export interface WorkbuddyAiAutoCheckinConfig {
  enabled: boolean;
  startTime: string; // HH:mm, e.g. "06:00"
  endTime: string; // HH:mm, e.g. "12:00"
  lastCheckedDate?: string; // "YYYY-MM-DD"
  accountSchedules?: Record<string, WorkbuddyAiAccountScheduleState>;
}

export const DEFAULT_WORKBUDDY_AI_AUTO_CHECKIN_CONFIG: WorkbuddyAiAutoCheckinConfig = {
  enabled: false,
  startTime: '06:00',
  endTime: '12:00',
};

export const WORKBUDDY_AI_AUTO_CHECKIN_CONFIG_CHANGED_EVENT =
  'workbuddy-ai-auto-checkin-config-changed';

export type WorkbuddyAiAutoCheckinCycleResult =
  | 'disabled'
  | 'platform_disabled'
  | 'waiting'
  | 'already_running'
  | 'no_accounts'
  | 'completed'
  | 'retry_scheduled'
  | 'partial_failed'
  | 'failed'
  | 'retry';

let cachedConfig: WorkbuddyAiAutoCheckinConfig | null = null;

function cacheConfigLocally(config: WorkbuddyAiAutoCheckinConfig, emitChange = false): void {
  cachedConfig = config;
  if (emitChange && typeof window !== 'undefined') {
    window.dispatchEvent(new Event(WORKBUDDY_AI_AUTO_CHECKIN_CONFIG_CHANGED_EVENT));
  }
}

export async function getWorkbuddyAiAutoCheckinConfigAsync(): Promise<WorkbuddyAiAutoCheckinConfig> {
  try {
    const config = await invoke<WorkbuddyAiAutoCheckinConfig>(
      'get_workbuddy_ai_auto_checkin_config',
    );
    cacheConfigLocally(config);
    return config;
  } catch (err) {
    console.warn('[WorkbuddyAiAutoCheckin] 从 Rust 端获取配置失败，使用本地缓存或默认值:', err);
    return getWorkbuddyAiAutoCheckinConfig();
  }
}

export function hasCachedWorkbuddyAiAutoCheckinConfig(): boolean {
  return cachedConfig !== null;
}

export function getWorkbuddyAiAutoCheckinConfig(): WorkbuddyAiAutoCheckinConfig {
  return cachedConfig ?? DEFAULT_WORKBUDDY_AI_AUTO_CHECKIN_CONFIG;
}

export async function saveWorkbuddyAiAutoCheckinConfigAsync(
  config: WorkbuddyAiAutoCheckinConfig,
): Promise<void> {
  if (typeof window === 'undefined') {
    cacheConfigLocally(config);
    return;
  }
  await invoke('save_workbuddy_ai_auto_checkin_config', { config });
  cacheConfigLocally(config, true);
}

export function isValidTimeString(timeStr: string): boolean {
  if (!/^\d{2}:\d{2}$/.test(timeStr)) {
    return false;
  }
  const [h, m] = timeStr.split(':').map(Number);
  return h >= 0 && h <= 23 && m >= 0 && m <= 59;
}

export function parseTimeToMinutes(timeStr: string): number {
  const parts = timeStr.split(':').map(Number);
  const h = parts[0] ?? 0;
  const m = parts[1] ?? 0;
  return h * 60 + m;
}

export function formatMinutesToTime(minutes: number): string {
  const h = Math.floor(minutes / 60) % 24;
  const m = minutes % 60;
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
}

export function normalizeWorkbuddyAiAutoCheckinCycleResult(
  raw: string,
): WorkbuddyAiAutoCheckinCycleResult {
  switch (raw) {
    case 'disabled':
    case 'platform_disabled':
    case 'waiting':
    case 'already_running':
    case 'no_accounts':
    case 'completed':
    case 'retry_scheduled':
    case 'partial_failed':
    case 'failed':
    case 'retry':
      return raw;
    default:
      return 'completed';
  }
}

export interface WorkbuddyAiAutoCheckinAccountDetail {
  accountId: string;
  email: string;
  status: 'success' | 'already_checked' | 'inactive' | 'pending' | 'failed';
  time?: string;
  resultCode?: string;
  modelId?: string;
  message?: string;
  credit?: number | null;
}

export interface WorkbuddyAiAutoCheckinLogRecord {
  id: string;
  timestamp: string;
  date: string;
  durationMs: number;
  totalAccounts: number;
  successCount: number;
  alreadyCheckedCount: number;
  inactiveCount?: number;
  pendingCount?: number;
  failedCount: number;
  status: 'success' | 'partial' | 'inactive' | 'pending' | 'failed' | 'no_accounts';
  details: WorkbuddyAiAutoCheckinAccountDetail[];
}

export const WORKBUDDY_AI_AUTO_CHECKIN_LOGS_CHANGED_EVENT =
  'workbuddy-ai-auto-checkin-logs-changed';

export function getWorkbuddyAiAutoCheckinLogsAsync(
  retry = false,
): Promise<LocalHistoryRead<WorkbuddyAiAutoCheckinLogRecord>> {
  return invoke<LocalHistoryRead<WorkbuddyAiAutoCheckinLogRecord>>(
    'get_workbuddy_ai_auto_checkin_logs',
    { retry },
  );
}

export async function clearWorkbuddyAiAutoCheckinLogs(): Promise<void> {
  await invoke('clear_workbuddy_ai_auto_checkin_logs');
}

export async function runWorkbuddyAiAutoCheckinCycleIfNeeded(
  force = false,
): Promise<WorkbuddyAiAutoCheckinCycleResult> {
  const res = await invoke<string>('run_workbuddy_ai_auto_checkin_now', { force });
  return normalizeWorkbuddyAiAutoCheckinCycleResult(res);
}
