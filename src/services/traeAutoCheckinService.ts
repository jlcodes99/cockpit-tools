/**
 * Trae CN 自动签到服务。
 * 调度与持久化在 Rust 后台（modules/trae_auto_checkin.rs），本文件只做
 * 配置本地缓存与 Tauri 命令桥接，结构对齐 workbuddyAutoCheckinService。
 */

import { invoke } from '@tauri-apps/api/core';

export interface TraeAccountScheduleState {
  scheduledDate: string; // "YYYY-MM-DD"
  scheduledMinute: number; // Minutes from midnight (0..1439)
  lastCheckedDate?: string; // "YYYY-MM-DD" when checked in
}

export interface TraeAutoCheckinConfig {
  enabled: boolean;
  startTime: string; // HH:mm, e.g. "06:00"
  endTime: string; // HH:mm, e.g. "12:00"
  lastCheckedDate?: string; // "YYYY-MM-DD"
  accountSchedules?: Record<string, TraeAccountScheduleState>;
  deviceIds?: Record<string, string>;
}

export const DEFAULT_TRAE_AUTO_CHECKIN_CONFIG: TraeAutoCheckinConfig = {
  enabled: false,
  startTime: '06:00',
  endTime: '12:00',
};

const CONFIG_KEY = 'agtools.trae.auto_checkin_config';
export const TRAE_AUTO_CHECKIN_CONFIG_CHANGED_EVENT = 'trae-auto-checkin-config-changed';
export const TRAE_AUTO_CHECKIN_LOGS_CHANGED_EVENT = 'trae-auto-checkin-logs-changed';
/** 后台签到成功后额度已刷新，前端应重取账号列表 */
export const TRAE_AUTO_CHECKIN_ACCOUNTS_CHANGED_EVENT = 'trae-auto-checkin-accounts-changed';

export type TraeAutoCheckinCycleResult = 'disabled' | 'waiting' | 'completed' | 'retry';

let cachedConfig: TraeAutoCheckinConfig | null = null;

function isValidTime(time: unknown): time is string {
  return typeof time === 'string' && /^([01]\d|2[0-3]):[0-5]\d$/.test(time);
}

// 配置变更统一走 Tauri 事件（Rust emit）；前端保存后的 UI 同步由调用方自行 setState
function cacheConfigLocally(config: TraeAutoCheckinConfig): void {
  cachedConfig = config;
  if (typeof window === 'undefined') {
    return;
  }
  try {
    localStorage.setItem(CONFIG_KEY, JSON.stringify(config));
  } catch (err) {
    console.warn('[TraeAutoCheckin] 本地缓存保存失败:', err);
  }
}

export async function getTraeAutoCheckinConfigAsync(): Promise<TraeAutoCheckinConfig> {
  try {
    const config = await invoke<TraeAutoCheckinConfig>('get_trae_auto_checkin_config');
    cacheConfigLocally(config);
    return config;
  } catch (err) {
    console.warn('[TraeAutoCheckin] 从 Rust 端获取配置失败，使用本地缓存或默认值:', err);
    return getTraeAutoCheckinConfig();
  }
}

export function getTraeAutoCheckinConfig(): TraeAutoCheckinConfig {
  if (cachedConfig) {
    return cachedConfig;
  }
  if (typeof window === 'undefined') {
    return DEFAULT_TRAE_AUTO_CHECKIN_CONFIG;
  }
  try {
    const raw = localStorage.getItem(CONFIG_KEY);
    if (!raw) {
      return DEFAULT_TRAE_AUTO_CHECKIN_CONFIG;
    }
    const parsed = JSON.parse(raw);
    const config: TraeAutoCheckinConfig = {
      enabled: typeof parsed.enabled === 'boolean' ? parsed.enabled : false,
      startTime: isValidTime(parsed.startTime) ? parsed.startTime : '06:00',
      endTime: isValidTime(parsed.endTime) ? parsed.endTime : '12:00',
      lastCheckedDate:
        typeof parsed.lastCheckedDate === 'string' ? parsed.lastCheckedDate : undefined,
      accountSchedules:
        typeof parsed.accountSchedules === 'object' && parsed.accountSchedules !== null
          ? parsed.accountSchedules
          : undefined,
      deviceIds:
        typeof parsed.deviceIds === 'object' && parsed.deviceIds !== null
          ? parsed.deviceIds
          : undefined,
    };
    cachedConfig = config;
    return config;
  } catch {
    return DEFAULT_TRAE_AUTO_CHECKIN_CONFIG;
  }
}

export async function saveTraeAutoCheckinConfigAsync(
  config: TraeAutoCheckinConfig,
): Promise<void> {
  if (typeof window === 'undefined') {
    cacheConfigLocally(config);
    return;
  }
  await invoke('save_trae_auto_checkin_config', { config });
  cacheConfigLocally(config);
}

export function parseTimeToMinutes(timeStr: string): number {
  const parts = timeStr.split(':').map(Number);
  const h = parts[0] ?? 0;
  const m = parts[1] ?? 0;
  return h * 60 + m;
}

export interface TraeAutoCheckinAccountDetail {
  accountId: string;
  email: string;
  status: 'success' | 'already_checked' | 'failed' | 'inactive';
  time?: string;
  message?: string;
}

export interface TraeAutoCheckinLogRecord {
  id: string;
  timestamp: string;
  date: string;
  durationMs: number;
  totalAccounts: number;
  successCount: number;
  alreadyCheckedCount: number;
  failedCount: number;
  status: 'success' | 'partial' | 'failed' | 'no_accounts';
  details: TraeAutoCheckinAccountDetail[];
}

export async function getTraeAutoCheckinLogsAsync(): Promise<TraeAutoCheckinLogRecord[]> {
  return await invoke<TraeAutoCheckinLogRecord[]>('get_trae_auto_checkin_logs');
}

export async function clearTraeAutoCheckinLogsAsync(): Promise<void> {
  await invoke('clear_trae_auto_checkin_logs');
}

export async function runTraeAutoCheckinCycleIfNeeded(
  force = false,
): Promise<TraeAutoCheckinCycleResult> {
  const res = await invoke<string>('run_trae_auto_checkin_now', { force });
  if (res === 'already_running') {
    return 'waiting';
  }
  if (res === 'disabled' || res === 'waiting' || res === 'completed' || res === 'retry') {
    return res as TraeAutoCheckinCycleResult;
  }
  return 'completed';
}
