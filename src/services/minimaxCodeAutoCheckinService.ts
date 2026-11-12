import type { LocalHistoryRead } from '../hooks/useLocalHistory';
/**
 * MiniMax Code 自动签到服务
 *
 * 由 Rust 持久化配置并负责实际签到，前端仅缓存当前配置。
 */

import { invoke } from '@tauri-apps/api/core';
import { createCheckinConfigMigration } from './checkinConfigMigration';

export interface MinimaxCodeAccountScheduleState {
  scheduledDate: string; // "YYYY-MM-DD"
  scheduledMinute: number; // Minutes from midnight (0..1439)
  lastCheckedDate?: string; // "YYYY-MM-DD" when checked in
}

export interface MinimaxCodeAutoCheckinConfig {
  enabled: boolean;
  startTime: string; // HH:mm, e.g. "06:00"
  endTime: string; // HH:mm, e.g. "12:00"
  lastCheckedDate?: string; // "YYYY-MM-DD"
  accountSchedules?: Record<string, MinimaxCodeAccountScheduleState>;
}

export const DEFAULT_MINIMAX_CODE_AUTO_CHECKIN_CONFIG: MinimaxCodeAutoCheckinConfig = {
  enabled: false,
  startTime: '06:00',
  endTime: '12:00',
};

const CONFIG_KEY = 'agtools.minimaxcode.auto_checkin_config';
export const MINIMAX_CODE_AUTO_CHECKIN_CONFIG_CHANGED_EVENT =
  'minimax-code-auto-checkin-config-changed';
const AUTO_CHECKIN_RETRY_DELAY_MS = 5 * 60 * 1000;
const AUTO_CHECKIN_IDLE_RECHECK_DELAY_MS = 60 * 60 * 1000;

export type MinimaxCodeAutoCheckinCycleResult = 'disabled' | 'waiting' | 'completed' | 'retry';

let cachedConfig: MinimaxCodeAutoCheckinConfig | null = null;

function isValidTime(time: unknown): time is string {
  return typeof time === 'string' && /^([01]\d|2[0-3]):[0-5]\d$/.test(time);
}

function cacheConfigLocally(config: MinimaxCodeAutoCheckinConfig, emitChange = false): void {
  cachedConfig = config;
  if (emitChange && typeof window !== 'undefined') {
    window.dispatchEvent(new Event(MINIMAX_CODE_AUTO_CHECKIN_CONFIG_CHANGED_EVENT));
  }
}

const migrateLegacyConfig = createCheckinConfigMigration(
  'migrate_minimax_code_auto_checkin_config',
  readLegacyConfig,
);

export async function getMinimaxCodeAutoCheckinConfigAsync(): Promise<MinimaxCodeAutoCheckinConfig> {
  try {
    const config = await migrateLegacyConfig()
      ?? await invoke<MinimaxCodeAutoCheckinConfig>('get_minimax_code_auto_checkin_config');
    cacheConfigLocally(config);
    return config;
  } catch (err) {
    console.warn('[MinimaxCodeAutoCheckin] 从 Rust 端获取配置失败，使用本地缓存或默认值:', err);
    return getMinimaxCodeAutoCheckinConfig();
  }
}

export function getMinimaxCodeAutoCheckinConfig(): MinimaxCodeAutoCheckinConfig {
  return cachedConfig ?? readLegacyConfig() ?? DEFAULT_MINIMAX_CODE_AUTO_CHECKIN_CONFIG;
}

function readLegacyConfig(): MinimaxCodeAutoCheckinConfig | undefined {
  if (typeof window === 'undefined') {
    return undefined;
  }
  try {
    const raw = localStorage.getItem(CONFIG_KEY);
    if (!raw) {
      return undefined;
    }
    const parsed = JSON.parse(raw);
    const config: MinimaxCodeAutoCheckinConfig = {
      enabled: typeof parsed.enabled === 'boolean' ? parsed.enabled : false,
      startTime: isValidTime(parsed.startTime) ? parsed.startTime : '06:00',
      endTime: isValidTime(parsed.endTime) ? parsed.endTime : '12:00',
      lastCheckedDate:
        typeof parsed.lastCheckedDate === 'string' ? parsed.lastCheckedDate : undefined,
      accountSchedules:
        typeof parsed.accountSchedules === 'object' && parsed.accountSchedules !== null
          ? parsed.accountSchedules
          : undefined,
    };
    return config;
  } catch {
    return undefined;
  }
}

export async function saveMinimaxCodeAutoCheckinConfigAsync(
  config: MinimaxCodeAutoCheckinConfig,
): Promise<void> {
  if (typeof window === 'undefined') {
    cacheConfigLocally(config);
    return;
  }
  await invoke('save_minimax_code_auto_checkin_config', { config });
  cacheConfigLocally(config, true);
}

export function saveMinimaxCodeAutoCheckinConfig(config: MinimaxCodeAutoCheckinConfig): void {
  void saveMinimaxCodeAutoCheckinConfigAsync(config).catch((err) => {
    console.warn('[MinimaxCodeAutoCheckin] 保存配置到 Rust 后端失败:', err);
  });
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

export function getTodayDateString(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

export function formatTimeOnly(date: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export function ensureAccountSchedules(
  config: MinimaxCodeAutoCheckinConfig,
  accounts: Array<{ id: string; email?: string }>,
): MinimaxCodeAutoCheckinConfig {
  const todayStr = getTodayDateString();
  const startMin = parseTimeToMinutes(config.startTime);
  let endMin = parseTimeToMinutes(config.endTime);
  if (endMin < startMin) {
    endMin = startMin;
  }
  const minRange = Math.max(0, endMin - startMin);

  const existingSchedules = config.accountSchedules || {};
  let changed = false;
  const updatedSchedules: Record<string, MinimaxCodeAccountScheduleState> = {
    ...existingSchedules,
  };

  for (const account of accounts) {
    const existing = existingSchedules[account.id];
    if (
      existing &&
      existing.scheduledDate === todayStr &&
      existing.scheduledMinute >= startMin &&
      existing.scheduledMinute <= endMin
    ) {
      continue;
    }

    const randomOffset = minRange > 0 ? Math.floor(Math.random() * (minRange + 1)) : 0;
    const scheduledMinute = startMin + randomOffset;

    updatedSchedules[account.id] = {
      scheduledDate: todayStr,
      scheduledMinute,
      lastCheckedDate: existing?.lastCheckedDate === todayStr ? todayStr : undefined,
    };
    changed = true;
  }

  if (changed) {
    const updatedConfig: MinimaxCodeAutoCheckinConfig = {
      ...config,
      accountSchedules: updatedSchedules,
    };
    saveMinimaxCodeAutoCheckinConfig(updatedConfig);
    return updatedConfig;
  }

  return config;
}

function getMillisecondsUntilNextLocalDay(now: Date): number {
  const nextDay = new Date(now);
  nextDay.setHours(24, 0, 1, 0);
  return Math.max(1_000, nextDay.getTime() - now.getTime());
}

export function getMinimaxCodeAutoCheckinNextDelayMs(
  result?: MinimaxCodeAutoCheckinCycleResult,
  accounts: Array<{ id: string }> = [],
): number {
  const config = getMinimaxCodeAutoCheckinConfig();
  if (!config.enabled) {
    return AUTO_CHECKIN_IDLE_RECHECK_DELAY_MS;
  }

  if (result === 'retry') {
    return AUTO_CHECKIN_RETRY_DELAY_MS;
  }

  const now = new Date();
  const todayStr = getTodayDateString();
  const updatedConfig = ensureAccountSchedules(config, accounts);
  const schedules = updatedConfig.accountSchedules || {};

  const currentMinute = now.getHours() * 60 + now.getMinutes();
  let nextScheduledMinute: number | null = null;

  for (const accId of Object.keys(schedules)) {
    const sch = schedules[accId];
    if (!sch) continue;
    if (sch.lastCheckedDate !== todayStr) {
      if (nextScheduledMinute === null || sch.scheduledMinute < nextScheduledMinute) {
        nextScheduledMinute = sch.scheduledMinute;
      }
    }
  }

  if (nextScheduledMinute === null) {
    return getMillisecondsUntilNextLocalDay(now);
  }

  if (currentMinute >= nextScheduledMinute) {
    return 1000;
  }

  const scheduledAt = new Date(now);
  scheduledAt.setHours(Math.floor(nextScheduledMinute / 60), nextScheduledMinute % 60, 0, 0);
  return Math.max(1_000, scheduledAt.getTime() - now.getTime());
}

export interface MinimaxCodeAutoCheckinAccountDetail {
  accountId: string;
  email: string;
  status: 'success' | 'already_checked' | 'failed' | 'inactive';
  time?: string;
  message?: string;
  credit?: number | null;
}

export interface MinimaxCodeAutoCheckinLogRecord {
  id: string;
  timestamp: string;
  date: string;
  durationMs: number;
  totalAccounts: number;
  successCount: number;
  alreadyCheckedCount: number;
  failedCount: number;
  status: 'success' | 'partial' | 'failed' | 'no_accounts';
  details: MinimaxCodeAutoCheckinAccountDetail[];
}

export const MINIMAX_CODE_AUTO_CHECKIN_LOGS_CHANGED_EVENT =
  'minimax-code-auto-checkin-logs-changed';

export function getMinimaxCodeAutoCheckinLogsAsync(retry = false): Promise<LocalHistoryRead<MinimaxCodeAutoCheckinLogRecord>> {
  return invoke<LocalHistoryRead<MinimaxCodeAutoCheckinLogRecord>>('get_minimax_code_auto_checkin_logs', { retry });
}

export async function clearMinimaxCodeAutoCheckinLogs(): Promise<void> {
  await invoke('clear_minimax_code_auto_checkin_logs');
}

export function formatFormattedTimestamp(date: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export async function runMinimaxCodeAutoCheckinCycleIfNeeded(
  force = false,
): Promise<MinimaxCodeAutoCheckinCycleResult> {
  const res = await invoke<string>('run_minimax_code_auto_checkin_now', { force });
  if (res === 'already_running') {
    return 'waiting';
  }
  if (res === 'disabled' || res === 'waiting' || res === 'completed' || res === 'retry') {
    return res as MinimaxCodeAutoCheckinCycleResult;
  }
  return 'completed';
}
